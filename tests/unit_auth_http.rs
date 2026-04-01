//! HTTP-level tests for auth: 401/403 responses, admin access control,
//! system graph blocking, insecure mode, and the full user management API.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hippo::auth::{GraphUserStore, USERS_GRAPH};
use hippo::config::Config;
use hippo::graph::GraphRegistry;
use hippo::state::AppState;
use hippo::testing::FakeLlm;
use tower::util::ServiceExt; // for oneshot

/// Build an AppState with auth enabled and a GraphUserStore backed by in-memory graph.
/// Returns (state, admin_api_key).
async fn test_state_with_auth() -> (Arc<AppState>, String) {
    let graphs = GraphRegistry::in_memory("test");
    let users_graph = graphs.get(USERS_GRAPH).await;
    let store = GraphUserStore::new(users_graph).await.unwrap();

    let admin_key = store
        .create_user("admin", "Admin", "admin", &["*".to_string()])
        .await
        .unwrap();

    let mut config = Config::test_default();
    config.auth.enabled = true;

    let (tx, rx) = tokio::sync::mpsc::channel(200);
    let (event_tx, _) = tokio::sync::broadcast::channel(256);

    let state = Arc::new(AppState {
        graphs: Some(graphs),
        llm: Arc::new(FakeLlm::new()),
        config,
        recent_nodes_tx: tx,
        recent_nodes_rx: Arc::new(tokio::sync::Mutex::new(rx)),
        recent_node_ids: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        checked_pairs: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        metrics: Arc::new(hippo::state::MetricsState::new()),
        credibility: Arc::new(tokio::sync::RwLock::new(
            hippo::credibility::CredibilityRegistry::new(),
        )),
        event_tx,
        user_store: Some(Arc::new(store)),
    });

    (state, admin_key)
}

/// Send a request to the router and return (status, body_string).
async fn send(state: Arc<AppState>, req: Request<Body>) -> (StatusCode, String) {
    let app = hippo::http::router(state);
    let resp: axum::http::Response<Body> = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).to_string())
}

fn json_post(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn json_post_auth(uri: &str, body: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {key}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get_auth(uri: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}

fn delete_auth(uri: &str, key: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("authorization", format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap()
}

// -- 401 Unauthorized ---------------------------------------------------------

#[tokio::test]
async fn no_auth_header_returns_401() {
    let (state, _) = test_state_with_auth().await;
    let req = json_post("/remember", r#"{"statement":"hello","source_agent":"t"}"#);
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("missing or invalid Authorization header"));
}

#[tokio::test]
async fn invalid_key_returns_401() {
    let (state, _) = test_state_with_auth().await;
    let req = json_post_auth(
        "/remember",
        r#"{"statement":"hello","source_agent":"t"}"#,
        "hippo_bogus",
    );
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("invalid API key"));
}

#[tokio::test]
async fn malformed_auth_header_returns_401() {
    let (state, _) = test_state_with_auth().await;
    let req = Request::builder()
        .method("POST")
        .uri("/remember")
        .header("content-type", "application/json")
        .header("authorization", "Basic dXNlcjpwYXNz") // Basic auth, not Bearer
        .body(Body::from(r#"{"statement":"hello","source_agent":"t"}"#))
        .unwrap();
    let (status, _) = send(state, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// -- Valid auth returns 200 ---------------------------------------------------

#[tokio::test]
async fn valid_admin_key_returns_200() {
    let (state, admin_key) = test_state_with_auth().await;
    let req = json_post_auth(
        "/remember",
        r#"{"statement":"Alice is great","source_agent":"t"}"#,
        &admin_key,
    );
    let (status, _) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
}

// -- Health/metrics don't require auth ----------------------------------------

#[tokio::test]
async fn health_does_not_require_auth() {
    let (state, _) = test_state_with_auth().await;
    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn metrics_does_not_require_auth() {
    let (state, _) = test_state_with_auth().await;
    let req = Request::builder()
        .method("GET")
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
}

// -- Admin endpoint access control --------------------------------------------

#[tokio::test]
async fn non_admin_cannot_create_users() {
    let (state, admin_key) = test_state_with_auth().await;

    // Create a regular user
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"bob","display_name":"Bob","role":"user","graphs":["test"]}"#,
        &admin_key,
    );
    let (status, body) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);
    let bob_key: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bob_key = bob_key["api_key"].as_str().unwrap();

    // Bob tries to create another user → 403
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"eve","display_name":"Eve","role":"user","graphs":[]}"#,
        bob_key,
    );
    let (status, body) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("admin access required"));
}

#[tokio::test]
async fn non_admin_cannot_list_users() {
    let (state, admin_key) = test_state_with_auth().await;

    // Create bob
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"bob","display_name":"Bob","role":"user","graphs":["test"]}"#,
        &admin_key,
    );
    let (_, body) = send(Arc::clone(&state), req).await;
    let bob_key: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bob_key = bob_key["api_key"].as_str().unwrap();

    // Bob lists users → 403
    let req = get_auth("/admin/users", bob_key);
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn non_admin_cannot_delete_users() {
    let (state, admin_key) = test_state_with_auth().await;

    // Create bob
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"bob","display_name":"Bob","role":"user","graphs":["test"]}"#,
        &admin_key,
    );
    let (_, body) = send(Arc::clone(&state), req).await;
    let bob_key: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bob_key = bob_key["api_key"].as_str().unwrap();

    // Bob tries to delete admin → 403
    let req = delete_auth("/admin/users/admin", bob_key);
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_can_list_users() {
    let (state, admin_key) = test_state_with_auth().await;

    let req = get_auth("/admin/users", &admin_key);
    let (status, body) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let users = parsed["users"].as_array().unwrap();
    assert!(users.iter().any(|u| u["user_id"] == "admin"));
}

// -- Admin key management endpoints -------------------------------------------

#[tokio::test]
async fn admin_key_lifecycle() {
    let (state, admin_key) = test_state_with_auth().await;

    // Create bob
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"bob","display_name":"Bob","role":"user","graphs":["test"]}"#,
        &admin_key,
    );
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);

    // Create a second key for bob
    let req = json_post_auth(
        "/admin/users/bob/keys",
        r#"{"label":"ci"}"#,
        &admin_key,
    );
    let (status, body) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);
    let ci_key: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(ci_key["label"], "ci");
    assert!(ci_key["api_key"].as_str().unwrap().starts_with("hippo_"));

    // List keys for bob → 2 keys
    let req = get_auth("/admin/users/bob/keys", &admin_key);
    let (status, body) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["keys"].as_array().unwrap().len(), 2);

    // Revoke the ci key
    let req = delete_auth("/admin/users/bob/keys/ci", &admin_key);
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);

    // List keys → only 1 remaining
    let req = get_auth("/admin/users/bob/keys", &admin_key);
    let (status, body) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["keys"].as_array().unwrap().len(), 1);
}

// -- System graph blocking ----------------------------------------------------

#[tokio::test]
async fn non_admin_blocked_from_system_graph() {
    let (state, admin_key) = test_state_with_auth().await;

    // Create bob with access to "test" only
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"bob","display_name":"Bob","role":"user","graphs":["test"]}"#,
        &admin_key,
    );
    let (_, body) = send(Arc::clone(&state), req).await;
    let bob_key: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bob_key = bob_key["api_key"].as_str().unwrap();

    // Bob tries to query hippo-users graph → 403
    let req = json_post_auth(
        "/context",
        r#"{"query":"users","graph":"hippo-users"}"#,
        bob_key,
    );
    let (status, body) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.contains("system graphs are not accessible"));

    // Bob tries admin-* graph → 403
    let req = json_post_auth(
        "/context",
        r#"{"query":"test","graph":"admin-config"}"#,
        bob_key,
    );
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// -- Graph ACL enforcement at endpoint level ----------------------------------

#[tokio::test]
async fn user_blocked_from_unauthorized_graph() {
    let (state, admin_key) = test_state_with_auth().await;

    // Create bob with access only to "mydb"
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"bob","display_name":"Bob","role":"user","graphs":["mydb"]}"#,
        &admin_key,
    );
    let (_, body) = send(Arc::clone(&state), req).await;
    let bob_key: serde_json::Value = serde_json::from_str(&body).unwrap();
    let bob_key = bob_key["api_key"].as_str().unwrap();

    // Bob can access default graph "test" → 403 (not in his ACL)
    let req = json_post_auth(
        "/remember",
        r#"{"statement":"hello","source_agent":"t"}"#,
        bob_key,
    );
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Bob can access "mydb" → 200
    let req = json_post_auth(
        "/remember",
        r#"{"statement":"hello","source_agent":"t","graph":"mydb"}"#,
        bob_key,
    );
    let (status, _) = send(Arc::clone(&state), req).await;
    assert_eq!(status, StatusCode::OK);
}

// -- Insecure mode ------------------------------------------------------------

#[tokio::test]
async fn insecure_mode_bypasses_auth() {
    let graphs = GraphRegistry::in_memory("test");
    let mut config = Config::test_default();
    config.auth.enabled = true;
    config.auth.insecure = true;

    let (tx, rx) = tokio::sync::mpsc::channel(200);
    let (event_tx, _) = tokio::sync::broadcast::channel(256);

    // Note: user_store is Some (auth enabled) but insecure overrides
    let users_graph = graphs.get(USERS_GRAPH).await;
    let store = GraphUserStore::new(users_graph).await.unwrap();
    store
        .create_user("admin", "Admin", "admin", &["*".to_string()])
        .await
        .unwrap();

    let state = Arc::new(AppState {
        graphs: Some(graphs),
        llm: Arc::new(FakeLlm::new()),
        config,
        recent_nodes_tx: tx,
        recent_nodes_rx: Arc::new(tokio::sync::Mutex::new(rx)),
        recent_node_ids: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        checked_pairs: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        metrics: Arc::new(hippo::state::MetricsState::new()),
        credibility: Arc::new(tokio::sync::RwLock::new(
            hippo::credibility::CredibilityRegistry::new(),
        )),
        event_tx,
        user_store: Some(Arc::new(store)),
    });

    // No auth header at all → still 200 (insecure mode)
    let req = json_post("/remember", r#"{"statement":"hello","source_agent":"t"}"#);
    let (status, _) = send(state, req).await;
    assert_eq!(status, StatusCode::OK);
}

// -- Duplicate user returns error ---------------------------------------------

#[tokio::test]
async fn create_duplicate_user_returns_400() {
    let (state, admin_key) = test_state_with_auth().await;

    // admin already exists from bootstrap
    let req = json_post_auth(
        "/admin/users",
        r#"{"user_id":"admin","display_name":"Admin2","role":"user","graphs":[]}"#,
        &admin_key,
    );
    let (status, _) = send(state, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
