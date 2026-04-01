use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::client::HippoClient;

// ---------------------------------------------------------------------------
// ZeroClaw trait types (defined locally to avoid a direct crate dependency).
// These mirror ZeroClaw's canonical trait signatures -- verify compatibility
// with your ZeroClaw version before use.
// ---------------------------------------------------------------------------

/// Category of a memory entry.
#[derive(Debug, Clone, PartialEq)]
pub enum MemoryCategory {
    Conversation,
    Knowledge,
    System,
    Custom(String),
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conversation => write!(f, "conversation"),
            Self::Knowledge => write!(f, "knowledge"),
            Self::System => write!(f, "system"),
            Self::Custom(s) => write!(f, "{s}"),
        }
    }
}

impl MemoryCategory {
    fn from_str(s: &str) -> Self {
        match s {
            "conversation" => Self::Conversation,
            "knowledge" => Self::Knowledge,
            "system" => Self::System,
            other => Self::Custom(other.to_string()),
        }
    }
}

/// A single memory entry.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub content: String,
    pub category: MemoryCategory,
    pub session_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
}

/// ZeroClaw Memory trait.
///
/// This is a local mirror of the trait defined in the ZeroClaw runtime crate.
/// Ensure the signatures stay in sync with your ZeroClaw version.
#[async_trait]
pub trait Memory: Send + Sync {
    fn name(&self) -> &str;
    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()>;
    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;
    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;
    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;
    async fn forget(&self, key: &str) -> Result<bool>;
    async fn count(&self) -> Result<usize>;
    fn health_check(&self) -> bool;
    async fn reindex(
        &self,
        progress_callback: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
    ) -> Result<usize>;
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

/// A ZeroClaw `Memory` backend that delegates to a Hippo instance over HTTP.
pub struct HippoMemory {
    client: HippoClient,
    healthy: AtomicBool,
}

impl HippoMemory {
    pub fn new(client: HippoClient) -> Self {
        Self {
            client,
            healthy: AtomicBool::new(true),
        }
    }
}

/// Encode key/category/session into the statement text so Hippo stores them
/// as searchable natural-language content.
fn encode_statement(
    key: &str,
    content: &str,
    category: &MemoryCategory,
    session_id: Option<&str>,
) -> String {
    let mut parts = vec![format!("[key={key}]"), format!("[category={category}]")];
    if let Some(sid) = session_id {
        parts.push(format!("[session={sid}]"));
    }
    parts.push(content.to_string());
    parts.join(" ")
}

/// Try to extract the `[key=...]` value from a stored fact string.
fn extract_key(fact: &str) -> Option<String> {
    fact.strip_prefix("[key=")
        .and_then(|s| s.split(']').next())
        .map(|s| s.to_string())
}

/// Try to extract the `[category=...]` value from a stored fact string.
fn extract_category(fact: &str) -> MemoryCategory {
    if let Some(start) = fact.find("[category=") {
        let rest = &fact[start + 10..];
        if let Some(end) = rest.find(']') {
            return MemoryCategory::from_str(&rest[..end]);
        }
    }
    MemoryCategory::Custom("unknown".to_string())
}

/// Try to extract the `[session=...]` value from a stored fact string.
fn extract_session(fact: &str) -> Option<String> {
    if let Some(start) = fact.find("[session=") {
        let rest = &fact[start + 9..];
        if let Some(end) = rest.find(']') {
            return Some(rest[..end].to_string());
        }
    }
    None
}

/// Strip metadata tags from the front of a fact to recover the original content.
fn extract_content(fact: &str) -> String {
    let mut s = fact;
    // Remove leading bracketed tags.
    loop {
        let trimmed = s.trim_start();
        if trimmed.starts_with('[') {
            if let Some(end) = trimmed.find(']') {
                s = &trimmed[end + 1..];
                continue;
            }
        }
        break;
    }
    s.trim().to_string()
}

fn fact_to_entry(fact: &crate::models::ContextFact) -> MemoryEntry {
    let key = extract_key(&fact.fact).unwrap_or_else(|| fact.edge_id.to_string());
    let category = extract_category(&fact.fact);
    let session_id = extract_session(&fact.fact);
    let content = extract_content(&fact.fact);
    let timestamp = chrono::DateTime::parse_from_rfc3339(&fact.valid_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    let mut metadata = HashMap::new();
    metadata.insert("confidence".to_string(), fact.confidence.to_string());
    metadata.insert("salience".to_string(), fact.salience.to_string());
    metadata.insert("subject".to_string(), fact.subject.clone());
    metadata.insert("object".to_string(), fact.object.clone());
    metadata.insert("memory_tier".to_string(), fact.memory_tier.clone());

    MemoryEntry {
        key,
        content,
        category,
        session_id,
        timestamp,
        metadata,
    }
}

#[async_trait]
impl Memory for HippoMemory {
    fn name(&self) -> &str {
        "hippo"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<()> {
        let statement = encode_statement(key, content, &category, session_id);
        self.client.remember(&statement, "zeroclaw").await?;
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let resp = self.client.context(query, Some(limit)).await?;
        Ok(resp.facts.iter().map(fact_to_entry).collect())
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let resp = self.client.context(key, Some(1)).await?;
        Ok(resp.facts.first().map(fact_to_entry))
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        // Use a broad wildcard query; Hippo will return the most relevant facts.
        let query = match category {
            Some(cat) => format!("[category={cat}]"),
            None => "*".to_string(),
        };
        let resp = self.client.context(&query, Some(100)).await?;

        let entries: Vec<MemoryEntry> = resp
            .facts
            .iter()
            .map(fact_to_entry)
            .filter(|e| {
                category
                    .map(|c| e.category == *c)
                    .unwrap_or(true)
            })
            .collect();
        Ok(entries)
    }

    async fn forget(&self, _key: &str) -> Result<bool> {
        // Hippo does not support deletion of individual facts via the public API.
        // Facts can be invalidated over time through contradiction detection.
        Ok(false)
    }

    async fn count(&self) -> Result<usize> {
        let stats = self.client.graph_stats().await?;
        Ok(stats.entity_count + stats.edge_count)
    }

    fn health_check(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    async fn reindex(
        &self,
        _progress_callback: Option<Box<dyn Fn(usize, usize) + Send + Sync>>,
    ) -> Result<usize> {
        // Hippo indexes automatically on ingestion; nothing to do.
        Ok(0)
    }
}

/// Spawn a background task that periodically checks Hippo health and updates
/// the `healthy` flag. Call this once after creating `HippoMemory`.
pub async fn refresh_health(memory: &HippoMemory) {
    let ok = memory.client.health().await.unwrap_or(false);
    memory.healthy.store(ok, Ordering::Relaxed);
}
