use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;

use crate::graph_backend::GraphBackend;
use crate::models::{Entity, EMBEDDING_DIM};

pub const AUDIT_GRAPH: &str = "admin-audit";

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditEntry {
    pub user_id: String,
    pub action: String,
    pub details: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditEntryResponse {
    pub id: String,
    pub user_id: String,
    pub action: String,
    pub details: String,
    pub timestamp: String,
}

pub struct AuditLog {
    tx: mpsc::Sender<AuditEntry>,
}

impl AuditLog {
    /// Spawn a background worker that writes audit entries to the graph.
    ///
    /// When all `AuditLog` clones are dropped the sender closes, causing the
    /// worker to drain remaining entries and exit.
    pub fn spawn(graph: Arc<dyn GraphBackend>) -> Self {
        let (tx, rx) = mpsc::channel::<AuditEntry>(256);
        tokio::spawn(audit_worker(graph, rx));
        Self { tx }
    }

    /// Queue an audit entry for writing. Non-async, never blocks callers.
    pub fn log(&self, entry: AuditEntry) {
        if let Err(e) = self.tx.try_send(entry) {
            tracing::warn!("audit log channel full or closed: {e}");
        }
    }
}

async fn audit_worker(graph: Arc<dyn GraphBackend>, mut rx: mpsc::Receiver<AuditEntry>) {
    while let Some(entry) = rx.recv().await {
        write_entry(&*graph, entry).await;
    }
    tracing::info!("audit log worker shutting down");
}

async fn write_entry(graph: &dyn GraphBackend, entry: AuditEntry) {
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
    if let Err(e) = graph.upsert_entity(&entity).await {
        tracing::warn!("failed to write audit log: {e}");
    }
}

/// Query audit entries from the audit graph.
pub async fn query_audit_log(
    graph: &dyn GraphBackend,
    filter_user: Option<&str>,
    filter_action: Option<&str>,
    limit: usize,
) -> anyhow::Result<Vec<AuditEntryResponse>> {
    let entities = graph.dump_all_entities().await?;
    let mut entries: Vec<AuditEntryResponse> = entities
        .into_iter()
        .filter(|e| e.entity_type == "_audit")
        .filter_map(|e| {
            let hint: serde_json::Value =
                serde_json::from_str(e.hint.as_deref()?).ok()?;
            let user_id = hint["user_id"].as_str()?.to_string();
            let action = hint["action"].as_str()?.to_string();
            let details = hint["details"].as_str().unwrap_or("").to_string();
            let timestamp = hint["timestamp"].as_str().unwrap_or("").to_string();

            if let Some(fu) = filter_user {
                if user_id != fu {
                    return None;
                }
            }
            if let Some(fa) = filter_action {
                if action != fa {
                    return None;
                }
            }

            Some(AuditEntryResponse {
                id: e.id.clone(),
                user_id,
                action,
                details,
                timestamp,
            })
        })
        .collect();

    // Newest first
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    entries.truncate(limit);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory_graph::InMemoryGraph;

    #[tokio::test]
    async fn audit_worker_writes_and_drains() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph.clone());

        audit.log(AuditEntry {
            user_id: "alice".into(),
            action: "remember".into(),
            details: "statement: hello world".into(),
        });

        audit.log(AuditEntry {
            user_id: "bob".into(),
            action: "ask".into(),
            details: "question: what is X?".into(),
        });

        // Drop the sender so the worker drains and exits
        drop(audit);

        // Give the worker a moment to finish writing
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let entities = graph.dump_all_entities().await.unwrap();
        assert_eq!(entities.len(), 2);
        assert!(entities.iter().all(|e| e.entity_type == "_audit"));
        assert!(entities.iter().any(|e| e.name.contains("alice")));
        assert!(entities.iter().any(|e| e.name.contains("bob")));
    }

    #[tokio::test]
    async fn audit_log_entry_fields() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph.clone());

        audit.log(AuditEntry {
            user_id: "alice".into(),
            action: "remember".into(),
            details: "statement: hello world".into(),
        });

        drop(audit);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let entities = graph.dump_all_entities().await.unwrap();
        assert_eq!(entities.len(), 1);

        let hint: serde_json::Value =
            serde_json::from_str(entities[0].hint.as_deref().unwrap()).unwrap();
        assert_eq!(hint["user_id"], "alice");
        assert_eq!(hint["action"], "remember");
        assert_eq!(hint["details"], "statement: hello world");
        assert!(hint["timestamp"].as_str().is_some());
    }

    #[tokio::test]
    async fn audit_log_does_not_panic_on_drop() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph);

        audit.log(AuditEntry {
            user_id: "bob".into(),
            action: "auth.failure".into(),
            details: "partial_key: hippo_abc...".into(),
        });

        drop(audit);
        // No panic — worker shuts down gracefully
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn query_audit_log_returns_entries() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph.clone());

        audit.log(AuditEntry {
            user_id: "alice".into(),
            action: "remember".into(),
            details: "stmt1".into(),
        });
        audit.log(AuditEntry {
            user_id: "bob".into(),
            action: "ask".into(),
            details: "q1".into(),
        });
        audit.log(AuditEntry {
            user_id: "alice".into(),
            action: "seed".into(),
            details: "entities: 5".into(),
        });

        drop(audit);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Unfiltered
        let all = query_audit_log(&*graph, None, None, 100).await.unwrap();
        assert_eq!(all.len(), 3);

        // Filter by user
        let alice = query_audit_log(&*graph, Some("alice"), None, 100).await.unwrap();
        assert_eq!(alice.len(), 2);
        assert!(alice.iter().all(|e| e.user_id == "alice"));

        // Filter by action
        let asks = query_audit_log(&*graph, None, Some("ask"), 100).await.unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].user_id, "bob");

        // Limit
        let limited = query_audit_log(&*graph, None, None, 2).await.unwrap();
        assert_eq!(limited.len(), 2);
    }
}
