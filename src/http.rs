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

struct PrettyJson(StatusCode, String);

impl IntoResponse for PrettyJson {
    fn into_response(self) -> axum::response::Response {
        let mut resp = self.1.into_response();
        *resp.status_mut() = self.0;
        resp.headers_mut().insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
        resp
    }
}

fn pretty_ok(value: impl serde::Serialize) -> PrettyJson {
    PrettyJson(StatusCode::OK, serde_json::to_string_pretty(&value).unwrap_or_default())
}

fn pretty_err(e: anyhow::Error) -> PrettyJson {
    PrettyJson(
        StatusCode::INTERNAL_SERVER_ERROR,
        serde_json::to_string_pretty(&ErrorResponse { error: e.to_string() }).unwrap_or_default(),
    )
}

fn bad_request(msg: impl Into<String>) -> PrettyJson {
    PrettyJson(
        StatusCode::BAD_REQUEST,
        serde_json::to_string_pretty(&ErrorResponse { error: msg.into() }).unwrap_or_default(),
    )
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
    type Rejection = PrettyJson;

    fn from_request(
        req: Request,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        async {
            let Json(value) = Json::<T>::from_request(req, state)
                .await
                .map_err(|e: JsonRejection| bad_request(e.body_text()))?;

            value.validate().map_err(bad_request)?;
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

use crate::models::{
    AdminSeedRequest, AdminSeedResponse, AskRequest,
    BatchRememberRequest, BatchRememberResponse, BatchRememberResult,
    ConsolidateRequest, ContextProgress, ContextRequest, ErrorResponse, HealthResponse,
    MemoryTierStats, ReflectRequest, RememberProgress, RememberRequest,
    SmartQueryRequest, TemporalContextRequest,
};
use crate::pipeline::{ask, consolidate, context, context_temporal, diagnose, maintain, query, reflect, remember, timeline};

#[derive(Debug, serde::Deserialize)]
struct GraphQuery {
    graph: Option<String>,
}
use crate::state::AppState;

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

async fn ask_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<AskRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    match ask::ask(&state, &*graph, req).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Ask error: {e}");
            pretty_err(e)
        }
    }
}

async fn query_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<SmartQueryRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;

    match query::smart_query(&state, &*graph, req).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Smart query error: {e}");
            pretty_err(e)
        }
    }
}

async fn remember_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<RememberRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;

    match remember::remember(&state, &*graph, req, None).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Remember error: {e}");
            pretty_err(e)
        }
    }
}

async fn remember_stream_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<RememberRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<RememberProgress>(32);
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;


    tokio::spawn(async move {
        match remember::remember(&state, &*graph, req, Some(tx.clone())).await {
            Ok(_) => {} // Complete event already sent by the pipeline
            Err(e) => {
                tracing::error!("Remember stream error: {e}");
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
) -> impl IntoResponse {
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

    pretty_ok(BatchRememberResponse {
        total,
        succeeded,
        failed,
        results,
    })
}

async fn context_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ContextRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    match context::context(&state, &*graph, req, None).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Context error: {e}");
            pretty_err(e)
        }
    }
}

async fn context_stream_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ContextRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = tokio::sync::mpsc::channel::<ContextProgress>(32);

    tokio::spawn(async move {
        let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
        match context::context(&state, &*graph, req, Some(tx.clone())).await {
            Ok(_) => {} // Done event already sent by the pipeline
            Err(e) => {
                tracing::error!("Context stream error: {e}");
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
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    match context_temporal::context_temporal(&state, &*graph, req).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Context temporal error: {e}");
            pretty_err(e)
        }
    }
}

async fn reflect_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ReflectRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    match reflect::reflect(&state, &*graph, req).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Reflect error: {e}");
            pretty_err(e)
        }
    }
}

async fn timeline_handler(
    State(state): State<Arc<AppState>>,
    Path(entity_name): Path<String>,
) -> impl IntoResponse {
    let graph = state.graph_registry().get_default().await;
    match timeline::timeline(&state, &*graph, &entity_name).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Timeline error: {e}");
            pretty_err(e)
        }
    }
}

async fn diagnose_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ContextRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    match diagnose::diagnose(&state, &*graph, req).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Diagnose error: {e}");
            pretty_err(e)
        }
    }
}

async fn graph_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GraphQuery>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(params.graph.as_deref()).await;
    match diagnose::graph_dump(&state, &*graph).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Graph dump error: {e}");
            pretty_err(e)
        }
    }
}

async fn maintain_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let graph = state.graph_registry().get_default().await;
    match maintain::run_once(&state, &*graph).await {
        Ok(()) => {
            pretty_ok(serde_json::json!({"status": "maintenance complete"}))
        }
        Err(e) => {
            tracing::error!("Maintenance error: {e}");
            pretty_err(e)
        }
    }
}

async fn consolidate_handler(
    State(state): State<Arc<AppState>>,
    ValidJson(req): ValidJson<ConsolidateRequest>,
) -> impl IntoResponse {
    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;
    match consolidate::consolidate(&state, &*graph, req).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Consolidation error: {e}");
            pretty_err(e)
        }
    }
}

async fn provenance_handler(
    State(state): State<Arc<AppState>>,
    Path(edge_id): Path<i64>,
) -> impl IntoResponse {
    let graph = state.graph_registry().get_default().await;
    match graph.get_provenance(edge_id).await {
        Ok(resp) => pretty_ok(resp),
        Err(e) => {
            tracing::error!("Provenance error: {e}");
            pretty_err(e)
        }
    }
}

async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.to_prometheus(),
    )
}

async fn sources_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let sources = state.credibility.read().await.list();
    pretty_ok(serde_json::json!({ "sources": sources }))
}

async fn memory_stats_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let graph = state.graph_registry().get_default().await;
    match graph.memory_tier_stats().await {
        Ok((working, long_term)) => pretty_ok(MemoryTierStats {
            working_count: working,
            long_term_count: long_term,
        }),
        Err(e) => {
            tracing::error!("Memory stats error: {e}");
            pretty_err(e)
        }
    }
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let graph = state.graph_registry().get_default().await;
    match graph.ping().await {
        Ok(()) => pretty_ok(HealthResponse {
            status: "ok".to_string(),
            graph: state.graph_registry().default_graph_name().to_string(),
        }),
        Err(e) => PrettyJson(
            StatusCode::SERVICE_UNAVAILABLE,
            serde_json::to_string_pretty(&ErrorResponse {
                error: format!("graph backend unavailable: {e}"),
            }).unwrap_or_default(),
        ),
    }
}

async fn admin_seed_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdminSeedRequest>,
) -> impl IntoResponse {
    use chrono::Utc;
    use crate::llm::pseudo_embed;
    use crate::models::{Entity, MemoryTier, Relation};

    let graph = state.graph_registry().resolve(req.graph.as_deref()).await;


    let mut entities_created = 0usize;
    let mut edges_created = 0usize;

    // Insert entities
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
        if let Err(err) = graph.upsert_entity(&entity).await {
            return PrettyJson(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::to_string_pretty(&crate::models::ErrorResponse {
                    error: format!("entity '{}': {err}", e.name),
                }).unwrap_or_default(),
            );
        }
        entities_created += 1;
    }

    // Insert edges
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
        if let Err(err) = graph.create_edge(&edge.subject_id, &edge.object_id, &relation).await {
            return PrettyJson(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::to_string_pretty(&crate::models::ErrorResponse {
                    error: format!("edge '{}': {err}", edge.fact),
                }).unwrap_or_default(),
            );
        }
        edges_created += 1;
    }

    pretty_ok(AdminSeedResponse {
        entities_created,
        edges_created,
    })
}

async fn graphs_list_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let graphs = state.graph_registry().list().await;
    pretty_ok(serde_json::json!({
        "default": state.graph_registry().default_graph_name(),
        "graphs": graphs,
    }))
}

async fn graphs_drop_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = state.graph_registry().drop_graph(&name).await {
        tracing::error!("Drop graph '{name}' error: {e}");
        return pretty_err(e);
    }

    // Clear in-memory state when dropping the default graph
    if name == state.graph_registry().default_graph_name() {
        state.recent_node_ids.write().await.clear();
        state.checked_pairs.write().await.clear();
        state.credibility.write().await.clear();
        state.metrics.reset();
    }

    pretty_ok(serde_json::json!({ "ok": true, "message": format!("Graph '{name}' dropped and reinitialised") }))
}
