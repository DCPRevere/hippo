use std::sync::Arc;

use chrono::Utc;

use crate::graph_backend::GraphBackend;
use crate::models::{Entity, EMBEDDING_DIM};

pub const AUDIT_GRAPH: &str = "admin-audit";

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditEntry {
    pub user_id: String,
    pub action: String,
    pub details: String,
}

pub struct AuditLog {
    graph: Arc<dyn GraphBackend>,
}

impl AuditLog {
    pub fn new(graph: Arc<dyn GraphBackend>) -> Self {
        Self { graph }
    }

    pub async fn log(&self, entry: AuditEntry) {
        let now = Utc::now();
        let entity = Entity {
            id: uuid::Uuid::new_v4().to_string(),
            name: format!(
                "{}:{}:{}",
                now.format("%Y%m%dT%H%M%S"),
                entry.user_id,
                entry.action
            ),
            entity_type: "_audit".to_string(),
            resolved: true,
            hint: Some(
                serde_json::json!({
                    "user_id": entry.user_id,
                    "action": entry.action,
                    "details": entry.details,
                    "timestamp": now.to_rfc3339(),
                })
                .to_string(),
            ),
            content: None,
            created_at: now,
            embedding: vec![0.0; EMBEDDING_DIM],
        };
        if let Err(e) = self.graph.upsert_entity(&entity).await {
            tracing::warn!("failed to write audit log: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory_graph::InMemoryGraph;

    #[tokio::test]
    async fn audit_log_writes_entity() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::new(graph.clone());

        audit
            .log(AuditEntry {
                user_id: "alice".into(),
                action: "remember".into(),
                details: "statement: hello world".into(),
            })
            .await;

        let entities = graph.dump_all_entities().await.unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].entity_type, "_audit");
        assert!(entities[0].name.contains("alice"));
        assert!(entities[0].name.contains("remember"));

        let hint: serde_json::Value =
            serde_json::from_str(entities[0].hint.as_deref().unwrap()).unwrap();
        assert_eq!(hint["user_id"], "alice");
        assert_eq!(hint["action"], "remember");
        assert_eq!(hint["details"], "statement: hello world");
    }

    #[tokio::test]
    async fn audit_log_does_not_panic_on_error() {
        // Use a graph that works, write an entry, then verify no panic.
        // The InMemoryGraph doesn't fail, but we can at least confirm the
        // code path handles the Result without panicking.
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::new(graph);

        // This should complete without panicking regardless of outcome
        audit
            .log(AuditEntry {
                user_id: "bob".into(),
                action: "auth.failure".into(),
                details: "partial_key: hippo_abc...".into(),
            })
            .await;
    }
}
