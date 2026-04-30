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
    /// Sender wrapped in `Mutex<Option<...>>` so `shutdown()` can drop it
    /// to close the channel without moving out of `&self`.
    tx: tokio::sync::Mutex<Option<mpsc::Sender<AuditEntry>>>,
    /// Worker task handle. Same `Mutex<Option<...>>` pattern so shutdown
    /// can `.await` it.
    worker: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl AuditLog {
    /// Spawn a background worker that writes audit entries to the graph.
    ///
    /// When all senders close, the worker drains remaining entries and
    /// exits. Call [`AuditLog::shutdown`] on graceful shutdown to wait
    /// for the drain to complete (otherwise tokio aborts the task when
    /// the runtime exits and tail entries are lost).
    pub fn spawn(graph: Arc<dyn GraphBackend>) -> Self {
        let (tx, rx) = mpsc::channel::<AuditEntry>(256);
        let worker = tokio::spawn(audit_worker(graph, rx));
        Self {
            tx: tokio::sync::Mutex::new(Some(tx)),
            worker: tokio::sync::Mutex::new(Some(worker)),
        }
    }

    /// Queue an audit entry for writing. Non-blocking on the hot path:
    /// uses `try_send`, so a full channel logs and drops rather than
    /// blocking the caller.
    pub fn log(&self, entry: AuditEntry) {
        // Try to grab the lock without blocking; if it's held by
        // shutdown, just log a warning and drop the entry.
        let guard = match self.tx.try_lock() {
            Ok(g) => g,
            Err(_) => {
                tracing::warn!("audit log: tx mutex held (likely during shutdown); dropping entry");
                return;
            }
        };
        let Some(tx) = guard.as_ref() else {
            tracing::warn!("audit log: already shut down; dropping entry");
            return;
        };
        if let Err(e) = tx.try_send(entry) {
            tracing::warn!("audit log channel full or closed: {e}");
        }
    }

    /// Close the channel and wait up to `timeout` for the worker to drain
    /// remaining entries. Idempotent: subsequent calls return immediately.
    ///
    /// Production callers should invoke this from main after the HTTP
    /// server returns and before the tokio runtime tears down.
    pub async fn shutdown(&self, timeout: std::time::Duration) {
        // Drop the sender to close the channel.
        {
            let mut tx_guard = self.tx.lock().await;
            tx_guard.take();
        }
        // Take the worker handle out and await it.
        let handle = {
            let mut guard = self.worker.lock().await;
            guard.take()
        };
        let Some(handle) = handle else {
            return; // already shut down
        };
        match tokio::time::timeout(timeout, handle).await {
            Ok(Ok(())) => tracing::info!("audit log drained cleanly"),
            Ok(Err(e)) => tracing::warn!("audit log worker panicked: {e}"),
            Err(_) => tracing::warn!("audit log drain timed out after {timeout:?}"),
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
            let hint: serde_json::Value = serde_json::from_str(e.hint.as_deref()?).ok()?;
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
    use crate::backends::InMemoryGraph;

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
        let alice = query_audit_log(&*graph, Some("alice"), None, 100)
            .await
            .unwrap();
        assert_eq!(alice.len(), 2);
        assert!(alice.iter().all(|e| e.user_id == "alice"));

        // Filter by action
        let asks = query_audit_log(&*graph, None, Some("ask"), 100)
            .await
            .unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].user_id, "bob");

        // Limit
        let limited = query_audit_log(&*graph, None, None, 2).await.unwrap();
        assert_eq!(limited.len(), 2);
    }

    /// shutdown() must drain queued entries before returning, without
    /// requiring a sleep on the test side.
    #[tokio::test]
    async fn shutdown_drains_pending_entries_deterministically() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph.clone());

        for i in 0..10 {
            audit.log(AuditEntry {
                user_id: format!("u{i}"),
                action: "test".into(),
                details: format!("detail {i}"),
            });
        }

        // shutdown should block until the worker has processed all 10
        // entries — no sleep needed.
        audit.shutdown(std::time::Duration::from_secs(2)).await;

        let entries = graph.dump_all_entities().await.unwrap();
        assert_eq!(
            entries.len(),
            10,
            "shutdown should have drained all 10 entries before returning"
        );
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph);
        audit.shutdown(std::time::Duration::from_secs(1)).await;
        // Second call returns immediately, no panic.
        audit.shutdown(std::time::Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn log_after_shutdown_drops_entry_without_panic() {
        let graph = Arc::new(InMemoryGraph::new(AUDIT_GRAPH));
        let audit = AuditLog::spawn(graph.clone());
        audit.shutdown(std::time::Duration::from_secs(1)).await;
        // Calling log() after shutdown must not panic.
        audit.log(AuditEntry {
            user_id: "ghost".into(),
            action: "post-shutdown".into(),
            details: "should be dropped".into(),
        });
        // And nothing extra should have been written.
        let entries = graph.dump_all_entities().await.unwrap();
        assert_eq!(entries.len(), 0);
    }
}
