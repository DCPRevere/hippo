//! User and API-key administration plus `/admin/audit`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};

use crate::auth::Auth;
use crate::error::AppError;
use crate::http::{internal, json_ok, JsonOk, ValidJson, Validate};
use crate::state::AppState;

const MAX_USER_ID_LEN: usize = 64;
const MAX_DISPLAY_NAME_LEN: usize = 128;
const MAX_LABEL_LEN: usize = 64;
const MAX_GRAPH_NAME_LEN: usize = 64;

/// `user_id`, key labels, and graph names share the same character set:
/// alphanumerics plus `-`, `_`, `.`. No whitespace, no control chars,
/// no quotes. This blocks injection attempts at the validation boundary.
fn is_safe_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
}

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

impl Validate for CreateUserRequest {
    fn validate(&self) -> Result<(), String> {
        if self.user_id.is_empty() || self.user_id.len() > MAX_USER_ID_LEN {
            return Err(format!("user_id must be 1..={MAX_USER_ID_LEN} characters"));
        }
        if !self.user_id.chars().all(is_safe_id_char) {
            return Err("user_id may only contain ASCII alphanumerics, '-', '_', '.'".into());
        }
        if self.display_name.trim().is_empty() || self.display_name.len() > MAX_DISPLAY_NAME_LEN {
            return Err(format!(
                "display_name must be non-empty and at most {MAX_DISPLAY_NAME_LEN} characters"
            ));
        }
        match self.role.as_str() {
            "admin" | "user" => {}
            _ => return Err("role must be one of: admin, user".into()),
        }
        for g in &self.graphs {
            if g == "*" {
                continue;
            }
            if g.is_empty() || g.len() > MAX_GRAPH_NAME_LEN {
                return Err(format!(
                    "graph names must be 1..={MAX_GRAPH_NAME_LEN} characters or '*'"
                ));
            }
            if !g.chars().all(is_safe_id_char) {
                return Err(
                    "graph names may only contain ASCII alphanumerics, '-', '_', '.'".into(),
                );
            }
        }
        Ok(())
    }
}

pub(crate) async fn admin_create_user_handler(
    State(state): State<Arc<AppState>>,
    Auth(user): Auth,
    ValidJson(req): ValidJson<CreateUserRequest>,
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

impl Validate for CreateKeyRequest {
    fn validate(&self) -> Result<(), String> {
        if self.label.is_empty() || self.label.len() > MAX_LABEL_LEN {
            return Err(format!("label must be 1..={MAX_LABEL_LEN} characters"));
        }
        if !self.label.chars().all(is_safe_id_char) {
            return Err("label may only contain ASCII alphanumerics, '-', '_', '.'".into());
        }
        Ok(())
    }
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
    ValidJson(req): ValidJson<CreateKeyRequest>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_create_user(user_id: &str, role: &str) -> CreateUserRequest {
        CreateUserRequest {
            user_id: user_id.into(),
            display_name: "Display".into(),
            role: role.into(),
            graphs: vec![],
        }
    }

    // -- CreateUserRequest --

    #[test]
    fn create_user_accepts_alphanumeric_id() {
        assert!(make_create_user("alice", "admin").validate().is_ok());
        assert!(make_create_user("bot-3.14", "user").validate().is_ok());
        assert!(make_create_user("svc_acct", "user").validate().is_ok());
    }

    #[test]
    fn create_user_rejects_empty_id() {
        let err = make_create_user("", "user").validate().unwrap_err();
        assert!(err.contains("user_id"));
    }

    #[test]
    fn create_user_rejects_overlong_id() {
        let long = "a".repeat(MAX_USER_ID_LEN + 1);
        let err = make_create_user(&long, "user").validate().unwrap_err();
        assert!(err.contains("user_id"));
    }

    #[test]
    fn create_user_rejects_unsafe_id_chars() {
        // Quotes, slashes, spaces, semicolons — anything that could escape
        // a backend string literal must bounce.
        for bad in [
            "alice'",
            "alice;DROP",
            "alice/admin",
            "alice bob",
            "alice\"x",
        ] {
            let err = make_create_user(bad, "user").validate().unwrap_err();
            assert!(err.contains("alphanumerics"), "{bad} not rejected: {err}");
        }
    }

    #[test]
    fn create_user_rejects_unknown_role() {
        let err = make_create_user("alice", "wizard").validate().unwrap_err();
        assert!(err.contains("role"));
    }

    #[test]
    fn create_user_accepts_star_graph_alias() {
        let req = CreateUserRequest {
            user_id: "alice".into(),
            display_name: "A".into(),
            role: "admin".into(),
            graphs: vec!["*".into()],
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_user_rejects_unsafe_graph_name() {
        let req = CreateUserRequest {
            user_id: "alice".into(),
            display_name: "A".into(),
            role: "user".into(),
            graphs: vec!["my graph".into()],
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn create_user_rejects_blank_display_name() {
        let req = CreateUserRequest {
            user_id: "alice".into(),
            display_name: "   ".into(),
            role: "user".into(),
            graphs: vec![],
        };
        assert!(req.validate().is_err());
    }

    // -- CreateKeyRequest --

    #[test]
    fn create_key_accepts_safe_label() {
        assert!(CreateKeyRequest {
            label: "default".into()
        }
        .validate()
        .is_ok());
        assert!(CreateKeyRequest {
            label: "ci-key.1".into()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn create_key_rejects_empty_label() {
        assert!(CreateKeyRequest { label: "".into() }.validate().is_err());
    }

    #[test]
    fn create_key_rejects_overlong_label() {
        let long = "x".repeat(MAX_LABEL_LEN + 1);
        assert!(CreateKeyRequest { label: long }.validate().is_err());
    }

    #[test]
    fn create_key_rejects_unsafe_chars() {
        for bad in ["key with space", "key/path", "key';--"] {
            assert!(
                CreateKeyRequest { label: bad.into() }.validate().is_err(),
                "{bad} not rejected"
            );
        }
    }
}
