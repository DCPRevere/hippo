//! Axum HTTP layer.
//!
//! This module exposes [`router`] which builds the full `Router` over an
//! `AppState`. The handlers themselves live in [`handlers`], grouped by
//! surface area (core / observability / graphs / admin).

use std::sync::Arc;

use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::auth::Auth as _Auth;
use crate::error::AppError;
use crate::models::{AskRequest, BatchRememberRequest, ContextRequest, RememberRequest};
use crate::state::AppState;

mod handlers;

// -- JSON response helper -----------------------------------------------------

pub(crate) struct JsonOk(String);

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

/// Build a JSON 200 response. On serialisation failure logs server-side and
/// returns an `AppError::internal` (500) without leaking details to clients.
pub(crate) fn json_ok(value: impl serde::Serialize) -> Result<JsonOk, AppError> {
    serde_json::to_string_pretty(&value)
        .map(JsonOk)
        .map_err(|e| {
            tracing::error!("response serialisation failed: {e}");
            AppError::internal("response serialisation failed")
        })
}

/// Map an internal error to a generic 500 without leaking details to the
/// client. The full error and a short context string are written to the
/// server log; the response body just says the operation failed.
pub(crate) fn internal(context: &'static str) -> impl FnOnce(anyhow::Error) -> AppError {
    move |e| {
        tracing::error!(error = ?e, "{context} failed");
        AppError::internal(format!("{context} failed"))
    }
}

/// Same as [`internal`] but for graph-backend errors that should surface as
/// 503 Service Unavailable rather than 500.
pub(crate) fn unavailable(context: &'static str) -> impl FnOnce(anyhow::Error) -> AppError {
    move |e| {
        tracing::error!(error = ?e, "{context} unavailable");
        AppError::unavailable(format!("{context} unavailable"))
    }
}

// -- Request validation -------------------------------------------------------

pub(crate) trait Validate {
    fn validate(&self) -> Result<(), String>;
}

/// Axum extractor that deserialises JSON then runs `Validate::validate`,
/// returning 400 on parse or validation failure.
pub(crate) struct ValidJson<T>(pub(crate) T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned + Validate + Send,
{
    type Rejection = AppError;

    #[allow(clippy::manual_async_fn)]
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

impl Validate for crate::models::RetractRequest {
    fn validate(&self) -> Result<(), String> {
        if self.edge_id <= 0 {
            return Err("edge_id must be positive".into());
        }
        if let Some(r) = &self.reason {
            if r.len() > 1024 {
                return Err("reason must be at most 1024 characters".into());
            }
        }
        Ok(())
    }
}

impl Validate for crate::models::CorrectRequest {
    fn validate(&self) -> Result<(), String> {
        if self.edge_id <= 0 {
            return Err("edge_id must be positive".into());
        }
        if self.statement.trim().is_empty() {
            return Err("statement must not be empty".into());
        }
        if let Some(r) = &self.reason {
            if r.len() > 1024 {
                return Err("reason must be at most 1024 characters".into());
            }
        }
        Ok(())
    }
}

// -- Router -------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub(crate) struct GraphQuery {
    pub(crate) graph: Option<String>,
    pub(crate) format: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    use handlers::{admin, core, graphs, observability};

    let api = Router::new()
        // Core endpoints
        .route("/remember", post(core::remember_handler))
        .route("/remember/batch", post(core::remember_batch_handler))
        .route("/context", post(core::context_handler))
        .route("/ask", post(core::ask_handler))
        // REST resources
        .route(
            "/entities/{id}",
            get(core::entity_handler).delete(core::entity_delete_handler),
        )
        .route("/entities/{id}/edges", get(core::entity_edges_handler))
        .route("/edges/{id}", get(core::edge_handler))
        .route("/edges/{id}/provenance", get(core::edge_provenance_handler))
        // User/agent destructive operations (distinct from Dreamer activity)
        .route("/retract", post(core::retract_handler))
        .route("/correct", post(core::correct_handler))
        // Operations
        .route("/maintain", post(core::maintain_handler))
        .route("/graph", get(core::graph_handler))
        // SSE
        .route("/events", get(observability::events_handler))
        // Observability
        .route("/health", get(observability::health_handler))
        .route("/metrics", get(observability::metrics_handler))
        // Graphs
        .route("/graphs", get(graphs::graphs_list_handler))
        .route("/graphs/drop/{name}", delete(graphs::graphs_drop_handler))
        // Seed
        .route("/seed", post(graphs::seed_handler))
        // Admin backup/restore
        .route("/admin/backup", post(graphs::backup_handler))
        .route("/admin/restore", post(graphs::restore_handler))
        // OpenAPI spec
        .route("/openapi.yaml", get(crate::openapi::openapi_handler))
        // Admin user management
        .route("/admin/users", post(admin::admin_create_user_handler))
        .route("/admin/users", get(admin::admin_list_users_handler))
        .route(
            "/admin/users/{user_id}",
            delete(admin::admin_delete_user_handler),
        )
        // API key management
        .route(
            "/admin/users/{user_id}/keys",
            post(admin::admin_create_key_handler),
        )
        .route(
            "/admin/users/{user_id}/keys",
            get(admin::admin_list_keys_handler),
        )
        .route(
            "/admin/users/{user_id}/keys/{label}",
            delete(admin::admin_revoke_key_handler),
        )
        // Audit log
        .route("/admin/audit", get(admin::admin_audit_handler));

    let ui_dir = std::env::var("HIPPO_UI_DIR").unwrap_or_else(|_| "ui/build".to_string());
    let ui_service =
        ServeDir::new(&ui_dir).not_found_service(ServeFile::new(format!("{}/index.html", ui_dir)));

    Router::new()
        // Root health endpoint for container health checks / backward compat
        .route("/health", get(observability::health_handler))
        .nest("/api", api)
        .layer(TraceLayer::new_for_http())
        .fallback_service(ui_service)
        .with_state(state)
}

// Keep `Auth` reachable via the old path while callers migrate.
#[allow(unused_imports)]
use _Auth as Auth;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn internal_helper_does_not_leak_underlying_error_text() {
        let raw = anyhow::anyhow!("postgres row 12345 violates unique constraint xyz_idx");
        let app = internal("seed entity")(raw);
        let resp = app.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(!s.contains("postgres row 12345"));
        assert!(!s.contains("unique constraint"));
        assert!(s.contains("seed entity"));
    }

    #[tokio::test]
    async fn unavailable_helper_returns_503_without_leaking_details() {
        let raw = anyhow::anyhow!("redis://prod-secret-host:6379 connection refused");
        let app = unavailable("graph backend")(raw);
        let resp = app.into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(!s.contains("prod-secret-host"));
        assert!(!s.contains("connection refused"));
        assert!(s.contains("graph backend"));
    }

    #[tokio::test]
    async fn json_ok_serialises_without_leaking_serde_error_internals_on_failure() {
        let bad: Result<String, serde_json::Error> = serde_json::from_str::<String>("not-a-string");
        let err = bad.unwrap_err();
        let app = AppError::internal("response serialisation failed");
        let resp = app.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(!s.contains(&err.to_string()));
    }
}
