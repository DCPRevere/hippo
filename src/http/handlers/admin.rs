//! User and API-key administration plus `/admin/audit`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;

use crate::auth::Auth;
use crate::error::AppError;
use crate::http::{internal, json_ok, JsonOk};
use crate::state::AppState;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CreateUserRequest {
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

pub(crate) async fn admin_create_user_handler(
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

    state.emit_audit(
        &user.user_id,
        "user.create",
        format!("user_id: {}", req.user_id),
    );

    json_ok(serde_json::json!({
        "user_id": req.user_id,
        "api_key": raw_key,
    }))
}

pub(crate) async fn admin_list_users_handler(
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
        .map_err(internal("list users"))?;

    json_ok(serde_json::json!({ "users": users }))
}

pub(crate) async fn admin_delete_user_handler(
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

    state.emit_audit(&user.user_id, "user.delete", format!("user_id: {user_id}"));

    json_ok(serde_json::json!({ "ok": true }))
}

// -- Admin API key management -------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub(crate) struct CreateKeyRequest {
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

pub(crate) async fn admin_create_key_handler(
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

    state.emit_audit(
        &user.user_id,
        "key.create",
        format!("user_id: {user_id}, label: {}", req.label),
    );

    json_ok(serde_json::json!({
        "user_id": user_id,
        "label": req.label,
        "api_key": raw_key,
    }))
}

pub(crate) async fn admin_list_keys_handler(
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

    json_ok(serde_json::json!({ "keys": keys }))
}

pub(crate) async fn admin_revoke_key_handler(
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

    state.emit_audit(
        &user.user_id,
        "key.revoke",
        format!("user_id: {user_id}, label: {label}"),
    );

    json_ok(serde_json::json!({ "ok": true }))
}

// -- Admin audit log ----------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub(crate) struct AuditQuery {
    user_id: Option<String>,
    action: Option<String>,
    limit: Option<usize>,
}

pub(crate) async fn admin_audit_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    Query(params): Query<AuditQuery>,
) -> Result<JsonOk, AppError> {
    if !user.is_admin() {
        return Err(AppError::forbidden("admin access required"));
    }

    let audit_graph = state.graph_registry().get(crate::audit::AUDIT_GRAPH).await;
    let limit = params.limit.unwrap_or(100).min(500);
    let entries = crate::audit::query_audit_log(
        &*audit_graph,
        params.user_id.as_deref(),
        params.action.as_deref(),
        limit,
    )
    .await
    .map_err(internal("query audit log"))?;

    json_ok(serde_json::json!({ "entries": entries }))
}
