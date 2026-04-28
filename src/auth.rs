use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use async_trait::async_trait;
#[cfg(not(target_arch = "wasm32"))]
use axum::extract::FromRequestParts;
#[cfg(not(target_arch = "wasm32"))]
use axum::http::request::Parts;
use chrono::Utc;
use tokio::sync::RwLock;

#[cfg(not(target_arch = "wasm32"))]
use crate::error::AppError;
use crate::graph_backend::GraphBackend;
use crate::models::Entity;
#[cfg(not(target_arch = "wasm32"))]
use crate::state::AppState;

// -- Constants ----------------------------------------------------------------

/// The graph name used to store user credentials.
pub const USERS_GRAPH: &str = "hippo-users";

/// Entity type for user records in the users graph.
const USER_ENTITY_TYPE: &str = "_user";

/// Returns true if the graph name is in a reserved system namespace.
pub fn is_system_graph(name: &str) -> bool {
    name.starts_with("hippo-") || name.starts_with("admin-")
}

// -- Types --------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub display_name: String,
    pub role: UserRole,
    pub allowed_graphs: GraphAcl,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UserRole {
    User,
    Admin,
}

#[derive(Debug, Clone)]
pub enum GraphAcl {
    All,
    Specific(HashSet<String>),
}

impl AuthenticatedUser {
    pub fn can_access_graph(&self, graph_name: &str) -> bool {
        match &self.allowed_graphs {
            GraphAcl::All => true,
            GraphAcl::Specific(set) => set.contains(graph_name),
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == UserRole::Admin
    }

    /// An anonymous user returned when auth is disabled.
    pub fn anonymous() -> Self {
        Self {
            user_id: "anonymous".to_string(),
            display_name: "Anonymous".to_string(),
            role: UserRole::Admin,
            allowed_graphs: GraphAcl::All,
        }
    }
}

pub use hippo_api::{ApiKeyInfo, UserInfo};

// -- UserStore trait -----------------------------------------------------------

#[async_trait]
pub trait UserStore: Send + Sync {
    /// Given a raw API key, return the authenticated user or None.
    async fn authenticate(&self, raw_key: &str) -> Option<AuthenticatedUser>;

    /// Downcast support for accessing concrete store implementations.
    fn as_any(&self) -> &dyn std::any::Any;
}

// -- Graph-backed store -------------------------------------------------------

/// A single API key associated with a user.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredApiKey {
    hash: String,
    label: String,
    created_at: String,
}

/// All data stored in the user entity's `hint` field.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct UserData {
    role: String,
    graphs: String,
    display_name: String,
    #[serde(default)]
    api_keys: Vec<StoredApiKey>,
}

struct CachedKey {
    hash: String,
    user: AuthenticatedUser,
}

pub struct GraphUserStore {
    graph: Arc<dyn GraphBackend>,
    /// Flattened cache: one entry per API key, all pointing to their user.
    cache: RwLock<Vec<CachedKey>>,
    /// Parsed user data keyed by user_id (entity name), for management ops.
    users: RwLock<HashMap<String, (String, UserData)>>, // user_id -> (entity_id, data)
}

impl GraphUserStore {
    pub async fn new(graph: Arc<dyn GraphBackend>) -> Result<Self> {
        let store = Self {
            graph,
            cache: RwLock::new(Vec::new()),
            users: RwLock::new(HashMap::new()),
        };
        store.refresh_cache().await?;
        Ok(store)
    }

    /// Reload all users from the graph into the in-memory cache.
    async fn refresh_cache(&self) -> Result<()> {
        let entities = self.graph.dump_all_entities().await?;
        let mut keys = Vec::new();
        let mut users = HashMap::new();

        for entity in &entities {
            if entity.entity_type != USER_ENTITY_TYPE || entity.name.starts_with("_deleted_") {
                continue;
            }
            let hint = match entity.hint.as_deref() {
                Some(h) => h,
                None => continue,
            };
            let data: UserData = match serde_json::from_str(hint) {
                Ok(d) => d,
                Err(_) => continue,
            };

            let role = match data.role.as_str() {
                "admin" => UserRole::Admin,
                _ => UserRole::User,
            };
            let allowed_graphs = parse_graphs_property(&data.graphs);
            let user = AuthenticatedUser {
                user_id: entity.name.clone(),
                display_name: data.display_name.clone(),
                role,
                allowed_graphs,
            };

            for key in &data.api_keys {
                keys.push(CachedKey {
                    hash: key.hash.clone(),
                    user: user.clone(),
                });
            }

            users.insert(entity.name.clone(), (entity.id.clone(), data));
        }

        *self.cache.write().await = keys;
        *self.users.write().await = users;
        Ok(())
    }

    /// Create a new user with an initial API key. Returns the raw key (shown once).
    pub async fn create_user(
        &self,
        user_id: &str,
        display_name: &str,
        role: &str,
        graphs: &[String],
    ) -> Result<String> {
        // Check the user doesn't already exist
        let existing = self.graph.fulltext_search_entities(user_id).await?;
        for e in &existing {
            if e.name == user_id && e.entity_type == USER_ENTITY_TYPE {
                anyhow::bail!("user '{user_id}' already exists");
            }
        }

        let (raw_key, hash) = generate_api_key()?;

        let data = UserData {
            role: role.to_string(),
            graphs: graphs.join(","),
            display_name: display_name.to_string(),
            api_keys: vec![StoredApiKey {
                hash,
                label: "default".to_string(),
                created_at: Utc::now().to_rfc3339(),
            }],
        };

        let entity = Entity {
            id: uuid::Uuid::new_v4().to_string(),
            name: user_id.to_string(),
            entity_type: USER_ENTITY_TYPE.to_string(),
            resolved: true,
            hint: Some(serde_json::to_string(&data)?),
            content: None,
            created_at: Utc::now(),
            embedding: vec![0.0; crate::models::EMBEDDING_DIM],
        };
        self.graph.upsert_entity(&entity).await?;

        self.refresh_cache().await?;
        Ok(raw_key)
    }

    /// Create an additional API key for an existing user. Returns the raw key.
    pub async fn create_api_key(&self, user_id: &str, label: &str) -> Result<String> {
        let users = self.users.read().await;
        let (entity_id, data) = users
            .get(user_id)
            .ok_or_else(|| anyhow::anyhow!("user '{user_id}' not found"))?;

        // Check label uniqueness within this user
        if data.api_keys.iter().any(|k| k.label == label) {
            anyhow::bail!("key label '{label}' already exists for user '{user_id}'");
        }

        let (raw_key, hash) = generate_api_key()?;

        let mut new_data = data.clone();
        new_data.api_keys.push(StoredApiKey {
            hash,
            label: label.to_string(),
            created_at: Utc::now().to_rfc3339(),
        });

        let entity_id = entity_id.clone();
        drop(users); // release lock before writing

        // Update the entity's hint field with new key list
        let entity = Entity {
            id: entity_id,
            name: user_id.to_string(),
            entity_type: USER_ENTITY_TYPE.to_string(),
            resolved: true,
            hint: Some(serde_json::to_string(&new_data)?),
            content: None,
            created_at: Utc::now(),
            embedding: vec![0.0; crate::models::EMBEDDING_DIM],
        };
        self.graph.upsert_entity(&entity).await?;

        self.refresh_cache().await?;
        Ok(raw_key)
    }

    /// Revoke an API key by label.
    pub async fn revoke_api_key(&self, user_id: &str, label: &str) -> Result<()> {
        let users = self.users.read().await;
        let (entity_id, data) = users
            .get(user_id)
            .ok_or_else(|| anyhow::anyhow!("user '{user_id}' not found"))?;

        if !data.api_keys.iter().any(|k| k.label == label) {
            anyhow::bail!("key label '{label}' not found for user '{user_id}'");
        }

        let mut new_data = data.clone();
        new_data.api_keys.retain(|k| k.label != label);

        let entity_id = entity_id.clone();
        drop(users);

        let entity = Entity {
            id: entity_id,
            name: user_id.to_string(),
            entity_type: USER_ENTITY_TYPE.to_string(),
            resolved: true,
            hint: Some(serde_json::to_string(&new_data)?),
            content: None,
            created_at: Utc::now(),
            embedding: vec![0.0; crate::models::EMBEDDING_DIM],
        };
        self.graph.upsert_entity(&entity).await?;

        self.refresh_cache().await?;
        Ok(())
    }

    /// List API keys for a user (labels and timestamps, no hashes).
    pub async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKeyInfo>> {
        let users = self.users.read().await;
        let (_, data) = users
            .get(user_id)
            .ok_or_else(|| anyhow::anyhow!("user '{user_id}' not found"))?;

        Ok(data
            .api_keys
            .iter()
            .map(|k| ApiKeyInfo {
                label: k.label.clone(),
                created_at: k.created_at.clone(),
            })
            .collect())
    }

    /// Delete a user by user_id (entity name).
    pub async fn delete_user(&self, user_id: &str) -> Result<()> {
        let entities = self.graph.fulltext_search_entities(user_id).await?;
        let found = entities
            .iter()
            .find(|e| e.name == user_id && e.entity_type == USER_ENTITY_TYPE);

        match found {
            Some(e) => {
                self.graph
                    .rename_entity(&e.id, &format!("_deleted_{user_id}"))
                    .await?;
                self.refresh_cache().await?;
                Ok(())
            }
            None => anyhow::bail!("user '{user_id}' not found"),
        }
    }

    /// List all users (without sensitive data).
    pub async fn list_users(&self) -> Result<Vec<UserInfo>> {
        let users = self.users.read().await;
        Ok(users
            .iter()
            .map(|(user_id, (_, data))| {
                let graphs = if data.graphs.is_empty() {
                    vec![]
                } else {
                    data.graphs
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect()
                };
                UserInfo {
                    user_id: user_id.clone(),
                    display_name: data.display_name.clone(),
                    role: data.role.clone(),
                    graphs,
                    key_count: data.api_keys.len(),
                }
            })
            .collect())
    }

    /// Check if any users exist.
    pub async fn has_users(&self) -> bool {
        !self.users.read().await.is_empty()
    }
}

#[async_trait]
impl UserStore for GraphUserStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn authenticate(&self, raw_key: &str) -> Option<AuthenticatedUser> {
        let argon2 = Argon2::default();
        let cache = self.cache.read().await;
        find_match_constant_time(cache.iter(), |ck| {
            PasswordHash::new(&ck.hash)
                .ok()
                .is_some_and(|hash| argon2.verify_password(raw_key.as_bytes(), &hash).is_ok())
        })
        .map(|ck| ck.user.clone())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn warn_once_insecure_mode() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            "AUTH BYPASS: hippo is running with auth.insecure = true. \
             All requests are treated as admin. Do not deploy in this state."
        );
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn warn_once_no_user_store() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            "AUTH BYPASS: no user store configured. \
             All requests are treated as admin. Configure HIPPO_AUTH_FILE or run hippo-cli to create a user."
        );
    });
}

/// Walks every entry in `entries` exactly once, returning the first match
/// without early exit. This guarantees auth time does not depend on the
/// position of a matching key in the cache.
///
/// Note: argon2 itself is constant-time per call, so the protection applies
/// across cache entries — not within a single hash verify.
fn find_match_constant_time<'a, T, I, F>(entries: I, verify: F) -> Option<&'a T>
where
    I: IntoIterator<Item = &'a T>,
    F: Fn(&T) -> bool,
{
    let mut found: Option<&T> = None;
    for entry in entries {
        if verify(entry) && found.is_none() {
            found = Some(entry);
        }
    }
    found
}

fn parse_graphs_property(s: &str) -> GraphAcl {
    if s.is_empty() {
        return GraphAcl::Specific(HashSet::new());
    }
    let items: Vec<&str> = s.split(',').map(|s| s.trim()).collect();
    if items.contains(&"*") {
        GraphAcl::All
    } else {
        GraphAcl::Specific(items.into_iter().map(|s| s.to_string()).collect())
    }
}

// -- In-memory store (for tests) ----------------------------------------------

pub struct InMemoryUserStore {
    users: HashMap<String, AuthenticatedUser>,
}

impl Default for InMemoryUserStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryUserStore {
    pub fn new() -> Self {
        Self {
            users: HashMap::new(),
        }
    }

    /// Add a user with a plaintext key (no hashing — test only).
    pub fn with_user(mut self, raw_key: &str, user: AuthenticatedUser) -> Self {
        self.users.insert(raw_key.to_string(), user);
        self
    }
}

#[async_trait]
impl UserStore for InMemoryUserStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn authenticate(&self, raw_key: &str) -> Option<AuthenticatedUser> {
        self.users.get(raw_key).cloned()
    }
}

// -- API key generation -------------------------------------------------------

/// Generate a random API key and its argon2id hash.
/// Returns `(raw_key, hash_string)`.
pub fn generate_api_key() -> Result<(String, String)> {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let raw_key = format!("hippo_{}", URL_SAFE_NO_PAD.encode(bytes));

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(raw_key.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("failed to hash API key: {e}"))?
        .to_string();

    Ok((raw_key, hash))
}

// -- Axum extractor -----------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
/// Axum extractor that provides the authenticated user.
///
/// When auth is disabled or insecure mode, returns an anonymous admin user.
/// When auth is enabled, validates the `Authorization: Bearer <key>` header.
pub struct Auth(pub AuthenticatedUser);

#[cfg(not(target_arch = "wasm32"))]
impl FromRequestParts<Arc<AppState>> for Auth {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        // --insecure mode: bypass auth entirely
        if state.config.auth.insecure {
            warn_once_insecure_mode();
            return Ok(Auth(AuthenticatedUser::anonymous()));
        }

        let store = match &state.user_store {
            Some(s) => s,
            None => {
                warn_once_no_user_store();
                return Ok(Auth(AuthenticatedUser::anonymous()));
            }
        };

        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        let raw_key = match auth_header {
            Some(h) if h.starts_with("Bearer ") => &h[7..],
            _ => {
                return Err(AppError::unauthorized(
                    "missing or invalid Authorization header",
                ))
            }
        };

        match store.authenticate(raw_key).await {
            Some(user) => {
                state.emit_audit(&user.user_id, "auth.success", "");
                if let Some(ref limiter) = state.rate_limiter {
                    if limiter.check(&user.user_id).is_err() {
                        return Err(AppError::too_many_requests("rate limit exceeded"));
                    }
                }
                Ok(Auth(user))
            }
            None => {
                let partial = if raw_key.len() > 10 {
                    format!("{}...", &raw_key[..10])
                } else {
                    raw_key.to_string()
                };
                state.emit_audit("unknown", "auth.failure", format!("partial_key: {partial}"));
                Err(AppError::unauthorized("invalid API key"))
            }
        }
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::InMemoryGraph;

    #[test]
    fn can_access_graph_checks() {
        let user = AuthenticatedUser {
            user_id: "test".into(),
            display_name: "Test".into(),
            role: UserRole::User,
            allowed_graphs: GraphAcl::Specific(["mydb".to_string()].into_iter().collect()),
        };
        assert!(user.can_access_graph("mydb"));
        assert!(!user.can_access_graph("other"));

        let admin = AuthenticatedUser::anonymous();
        assert!(admin.can_access_graph("anything"));
    }

    #[test]
    fn system_graph_detection() {
        assert!(is_system_graph("hippo-users"));
        assert!(is_system_graph("hippo-audit"));
        assert!(is_system_graph("admin-config"));
        assert!(!is_system_graph("my-graph"));
        assert!(!is_system_graph("hippo"));
    }

    #[test]
    fn parse_graphs_property_tests() {
        assert!(matches!(parse_graphs_property("*"), GraphAcl::All));
        assert!(matches!(parse_graphs_property(""), GraphAcl::Specific(s) if s.is_empty()));
        if let GraphAcl::Specific(s) = parse_graphs_property("a,b,c") {
            assert_eq!(s.len(), 3);
            assert!(s.contains("a"));
        } else {
            panic!("expected Specific");
        }
    }

    #[tokio::test]
    async fn generate_and_verify_api_key() {
        let (raw_key, hash) = generate_api_key().unwrap();
        assert!(raw_key.starts_with("hippo_"));

        let argon2 = Argon2::default();
        let parsed = PasswordHash::new(&hash).unwrap();
        assert!(argon2.verify_password(raw_key.as_bytes(), &parsed).is_ok());
        assert!(argon2.verify_password(b"wrong-key", &parsed).is_err());
    }

    #[tokio::test]
    async fn in_memory_store_authenticates() {
        let user = AuthenticatedUser {
            user_id: "alice".into(),
            display_name: "Alice".into(),
            role: UserRole::User,
            allowed_graphs: GraphAcl::All,
        };
        let store = InMemoryUserStore::new().with_user("my-secret-key", user);

        let result = store.authenticate("my-secret-key").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().user_id, "alice");

        assert!(store.authenticate("wrong-key").await.is_none());
    }

    #[tokio::test]
    async fn graph_user_store_create_and_authenticate() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        assert!(!store.has_users().await);

        let raw_key = store
            .create_user("alice", "Alice", "admin", &["*".to_string()])
            .await
            .unwrap();
        assert!(raw_key.starts_with("hippo_"));
        assert!(store.has_users().await);

        let user = store.authenticate(&raw_key).await;
        assert!(user.is_some());
        let user = user.unwrap();
        assert_eq!(user.user_id, "alice");
        assert_eq!(user.display_name, "Alice");
        assert_eq!(user.role, UserRole::Admin);
        assert!(user.can_access_graph("anything"));

        assert!(store.authenticate("hippo_wrong").await.is_none());
    }

    #[tokio::test]
    async fn graph_user_store_list_and_delete() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        store
            .create_user("alice", "Alice", "admin", &["*".to_string()])
            .await
            .unwrap();
        store
            .create_user("bob", "Bob", "user", &["default".to_string()])
            .await
            .unwrap();

        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 2);

        store.delete_user("bob").await.unwrap();

        let users = store.list_users().await.unwrap();
        assert_eq!(users.len(), 1);
        assert!(users.iter().any(|u| u.user_id == "alice"));
    }

    #[tokio::test]
    async fn graph_user_store_duplicate_user_rejected() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        store
            .create_user("alice", "Alice", "admin", &["*".to_string()])
            .await
            .unwrap();

        let result = store.create_user("alice", "Alice Again", "user", &[]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn graph_user_store_graph_acl() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        let raw_key = store
            .create_user(
                "bob",
                "Bob",
                "user",
                &["mydb".to_string(), "shared".to_string()],
            )
            .await
            .unwrap();

        let user = store.authenticate(&raw_key).await.unwrap();
        assert!(user.can_access_graph("mydb"));
        assert!(user.can_access_graph("shared"));
        assert!(!user.can_access_graph("secret"));
    }

    #[tokio::test]
    async fn multiple_api_keys_per_user() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        let key1 = store
            .create_user("alice", "Alice", "admin", &["*".to_string()])
            .await
            .unwrap();

        // Create a second key
        let key2 = store.create_api_key("alice", "ci").await.unwrap();
        assert_ne!(key1, key2);

        // Both keys authenticate as alice
        let u1 = store.authenticate(&key1).await.unwrap();
        let u2 = store.authenticate(&key2).await.unwrap();
        assert_eq!(u1.user_id, "alice");
        assert_eq!(u2.user_id, "alice");

        // List keys shows both
        let keys = store.list_api_keys("alice").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.iter().any(|k| k.label == "default"));
        assert!(keys.iter().any(|k| k.label == "ci"));

        // Revoke key2
        store.revoke_api_key("alice", "ci").await.unwrap();

        // key2 no longer works
        assert!(store.authenticate(&key2).await.is_none());
        // key1 still works
        assert!(store.authenticate(&key1).await.is_some());

        // Only one key left
        let keys = store.list_api_keys("alice").await.unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].label, "default");
    }

    #[tokio::test]
    async fn duplicate_key_label_rejected() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        store
            .create_user("alice", "Alice", "admin", &["*".to_string()])
            .await
            .unwrap();

        // "default" label already exists from create_user
        let result = store.create_api_key("alice", "default").await;
        assert!(result.is_err());
    }

    // ---- Added coverage ----

    #[test]
    fn anonymous_user_is_admin_with_all_graphs() {
        let u = AuthenticatedUser::anonymous();
        assert!(u.is_admin());
        assert!(u.can_access_graph("anything"));
        assert!(u.can_access_graph("hippo-users"));
    }

    #[test]
    fn graph_acl_all_grants_every_graph() {
        let u = AuthenticatedUser {
            user_id: "x".into(),
            display_name: "X".into(),
            role: UserRole::User,
            allowed_graphs: GraphAcl::All,
        };
        assert!(u.can_access_graph(""));
        assert!(u.can_access_graph("totally-unrelated"));
    }

    #[test]
    fn graph_acl_specific_empty_set_denies_all() {
        let u = AuthenticatedUser {
            user_id: "x".into(),
            display_name: "X".into(),
            role: UserRole::User,
            allowed_graphs: GraphAcl::Specific(HashSet::new()),
        };
        assert!(!u.can_access_graph("any"));
    }

    #[test]
    fn parse_graphs_with_star_anywhere_grants_all() {
        // `*` mixed with named graphs still resolves to All — documents
        // current behaviour so a stricter parse regresses loudly.
        let acl = parse_graphs_property("a, b, *");
        assert!(matches!(acl, GraphAcl::All));
    }

    #[test]
    fn parse_graphs_trims_whitespace() {
        if let GraphAcl::Specific(s) = parse_graphs_property("  a , b  , c") {
            assert!(s.contains("a"));
            assert!(s.contains("b"));
            assert!(s.contains("c"));
            assert!(!s.contains(" a "));
        } else {
            panic!("expected Specific");
        }
    }

    #[test]
    fn user_role_admin_only_when_role_is_admin() {
        let admin = AuthenticatedUser {
            user_id: "a".into(),
            display_name: "A".into(),
            role: UserRole::Admin,
            allowed_graphs: GraphAcl::All,
        };
        let user = AuthenticatedUser {
            user_id: "b".into(),
            display_name: "B".into(),
            role: UserRole::User,
            allowed_graphs: GraphAcl::All,
        };
        assert!(admin.is_admin());
        assert!(!user.is_admin());
    }

    #[test]
    fn generate_api_key_returns_unique_keys() {
        // 8 samples is enough to assert uniqueness without paying argon2 cost
        // many times (argon2id is intentionally ~10ms per call).
        let mut keys = std::collections::HashSet::new();
        for _ in 0..8 {
            let (k, _) = generate_api_key().unwrap();
            assert!(keys.insert(k), "duplicate key generated");
        }
    }

    #[test]
    fn generate_api_key_hash_is_argon2_format() {
        let (_, hash) = generate_api_key().unwrap();
        // argon2 PHC format: $argon2id$v=19$m=...,t=...,p=...$<salt>$<hash>
        assert!(hash.starts_with("$argon2"), "hash = {}", hash);
        assert_eq!(hash.matches('$').count(), 5);
    }

    #[test]
    fn generate_api_key_raw_does_not_appear_in_hash() {
        // Sanity: even a substring of the raw key should not leak into the hash.
        let (raw, hash) = generate_api_key().unwrap();
        let body = raw.trim_start_matches("hippo_");
        assert!(!hash.contains(body));
    }

    #[tokio::test]
    async fn in_memory_store_returns_none_for_unknown_key() {
        let store = InMemoryUserStore::new();
        assert!(store.authenticate("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn graph_store_unknown_role_string_defaults_to_user() {
        // create_user with a role string the parser doesn't recognise should
        // fall back to UserRole::User, not crash. Documents current behaviour.
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();

        let raw = store
            .create_user("z", "Z", "wizard", &["*".to_string()])
            .await
            .unwrap();
        let user = store.authenticate(&raw).await.unwrap();
        assert_eq!(user.role, UserRole::User);
    }

    #[test]
    fn find_match_constant_time_visits_every_entry_when_first_matches() {
        let entries = ["a", "b", "c", "d"];
        let calls = std::cell::Cell::new(0);
        let m = find_match_constant_time(entries.iter(), |e| {
            calls.set(calls.get() + 1);
            *e == "a"
        });
        assert_eq!(m, Some(&"a"));
        assert_eq!(
            calls.get(),
            4,
            "expected all entries visited, got {}",
            calls.get()
        );
    }

    #[test]
    fn find_match_constant_time_visits_every_entry_when_last_matches() {
        let entries = ["a", "b", "c", "d"];
        let calls = std::cell::Cell::new(0);
        let m = find_match_constant_time(entries.iter(), |e| {
            calls.set(calls.get() + 1);
            *e == "d"
        });
        assert_eq!(m, Some(&"d"));
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn find_match_constant_time_returns_none_when_nothing_matches() {
        let entries = ["a", "b", "c"];
        let calls = std::cell::Cell::new(0);
        let m = find_match_constant_time(entries.iter(), |_| {
            calls.set(calls.get() + 1);
            false
        });
        assert!(m.is_none());
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn warn_once_helpers_do_not_panic_when_called_repeatedly() {
        // Both helpers use std::sync::Once and must be safe across calls.
        for _ in 0..3 {
            warn_once_insecure_mode();
            warn_once_no_user_store();
        }
    }

    #[test]
    fn find_match_constant_time_returns_first_when_multiple_match() {
        let entries = ["a", "b", "c"];
        let m = find_match_constant_time(entries.iter(), |e| *e == "b" || *e == "c");
        assert_eq!(m, Some(&"b"));
    }

    #[tokio::test]
    async fn revoking_only_key_disables_user() {
        let graph = Arc::new(InMemoryGraph::new(USERS_GRAPH));
        let store = GraphUserStore::new(graph).await.unwrap();
        let raw = store
            .create_user("solo", "Solo", "user", &["*".to_string()])
            .await
            .unwrap();
        store.revoke_api_key("solo", "default").await.unwrap();
        assert!(store.authenticate(&raw).await.is_none());
    }
}
