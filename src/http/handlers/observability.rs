//! Observability endpoints: `/health`, `/metrics`, `/events` (SSE).

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use futures::Stream;
use tokio_stream::StreamExt as _;

use crate::error::AppError;
use crate::http::{json_ok, unavailable, GraphQuery, JsonOk};
use crate::models::HealthResponse;
use crate::state::AppState;

pub(crate) async fn health_handler(State(state): State<Arc<AppState>>) -> Result<JsonOk, AppError> {
    let graph = state.graph_registry().get_default().await;
    graph.ping().await.map_err(unavailable("graph backend"))?;
    json_ok(HealthResponse {
        status: "ok".to_string(),
        graph: state.graph_registry().default_graph_name().to_string(),
    })
}

pub(crate) async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.to_prometheus(),
    )
}

// -- SSE ----------------------------------------------------------------------

pub(crate) async fn events_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<GraphQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let graph_filter = params.graph;

    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(move |result| {
        match result {
            Ok(event) => {
                // Apply optional graph filter
                if let Some(ref g) = graph_filter {
                    if event.graph() != g {
                        return None;
                    }
                }
                let event_name = event.event_name().to_string();
                match serde_json::to_string(&event) {
                    Ok(data) => Some(Ok(Event::default().event(event_name).data(data))),
                    Err(e) => {
                        tracing::warn!(?e, "skipping unserialisable SSE event");
                        None
                    }
                }
            }
            // Skip lagged messages
            Err(_) => None,
        }
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

// -- Admin --------------------------------------------------------------------
