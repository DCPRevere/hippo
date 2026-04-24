#[cfg(not(target_arch = "wasm32"))]
use crate::error::GraphConnectError;
#[cfg(not(target_arch = "wasm32"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use async_trait::async_trait;
#[cfg(not(target_arch = "wasm32"))]
use chrono::{DateTime, Utc};
#[cfg(not(target_arch = "wasm32"))]
use falkordb::{AsyncGraph, FalkorClientBuilder, FalkorConnectionInfo, FalkorValue};
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashSet;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::LazyLock;
use tokio::sync::Mutex;

#[cfg(not(target_arch = "wasm32"))]
static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "my", "your", "his", "her", "its", "our", "their", "i", "me", "we", "you", "he", "she",
        "they", "it", "to", "of", "in", "for", "on", "with", "at", "by", "from", "and", "or",
        "but", "not", "no", "about", "what", "where", "when", "who", "how", "which", "that",
        "this", "these", "those",
    ]
    .into_iter()
    .collect()
});

use crate::graph_backend::GraphBackend;
#[cfg(not(target_arch = "wasm32"))]
use crate::models::{EdgeRow, EntityRow, SupersessionRecord, EMBEDDING_DIM};

#[cfg(not(target_arch = "wasm32"))]
const DEFAULT_POOL_SIZE: usize = 4;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct GraphClient {
    pool: Arc<Vec<Mutex<AsyncGraph>>>,
    next: Arc<std::sync::atomic::AtomicUsize>,
    graph_name: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl GraphClient {
    fn conn(&self) -> &Mutex<AsyncGraph> {
        let idx = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.pool.len();
        &self.pool[idx]
    }
}

type GraphFactory = Box<dyn Fn(&str) -> Arc<dyn GraphBackend> + Send + Sync>;

pub struct GraphRegistry {
    factory: GraphFactory,
    default_graph: String,
    graphs: Mutex<HashMap<String, Arc<dyn GraphBackend>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl GraphRegistry {
    pub async fn connect(connection_string: &str, default_graph: &str) -> Result<Self> {
        let info: FalkorConnectionInfo = connection_string.try_into().map_err(|e| {
            GraphConnectError::new(format!("invalid FalkorDB connection string: {e}"))
        })?;
        let client = FalkorClientBuilder::new_async()
            .with_connection_info(info)
            .build()
            .await
            .map_err(|e| GraphConnectError::new(format!("failed to connect to FalkorDB: {e}")))?;

        let registry = Self {
            factory: Box::new(move |name: &str| {
                let pool: Vec<Mutex<AsyncGraph>> = (0..DEFAULT_POOL_SIZE)
                    .map(|_| Mutex::new(client.select_graph(name)))
                    .collect();
                let graph_client = GraphClient {
                    pool: Arc::new(pool),
                    next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                    graph_name: name.to_string(),
                };
                Arc::new(graph_client) as Arc<dyn GraphBackend>
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };

        // Pre-create the default graph entry
        registry.get(default_graph).await;

        Ok(registry)
    }

    /// Create a registry backed by SQLite databases on disk.
    /// Each graph gets its own file: `{base_dir}/{graph_name}.db`.
    pub fn sqlite(default_graph: &str, base_path: String) -> Self {
        Self {
            factory: Box::new(move |name: &str| {
                let path = if base_path.is_empty() {
                    std::path::PathBuf::from(format!("{name}.db"))
                } else {
                    std::path::PathBuf::from(&base_path)
                };
                let graph = crate::sqlite_graph::SqliteGraph::open(name, &path)
                    .expect("failed to open SQLite graph");
                Arc::new(graph) as Arc<dyn GraphBackend>
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        }
    }

    /// Create a registry backed by PostgreSQL.
    /// All graphs share the same Postgres instance, distinguished by `graph_name` column.
    pub async fn postgres(connection_string: &str, default_graph: &str) -> Result<Self> {
        // Verify connectivity
        let test =
            crate::postgres_graph::PostgresGraph::new(connection_string, default_graph).await?;
        test.ping().await?;

        let conn_str = connection_string.to_string();
        let registry = Self {
            factory: Box::new(move |name: &str| {
                let pool = futures::executor::block_on(crate::postgres_graph::PostgresGraph::new(
                    &conn_str, name,
                ))
                .expect("failed to connect to PostgreSQL");
                Arc::new(pool) as Arc<dyn GraphBackend>
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };

        registry.get(default_graph).await;

        Ok(registry)
    }

    /// Create a registry backed by Qdrant vector database.
    pub async fn qdrant(url: &str, default_graph: &str) -> Result<Self> {
        // Verify connectivity
        let test = crate::qdrant_graph::QdrantGraph::new(url, default_graph).await?;
        test.ping().await?;

        let url_owned = url.to_string();
        let registry = Self {
            factory: Box::new(move |name: &str| {
                let graph = futures::executor::block_on(crate::qdrant_graph::QdrantGraph::new(
                    &url_owned, name,
                ))
                .expect("failed to connect to Qdrant");
                Arc::new(graph) as Arc<dyn GraphBackend>
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };

        registry.get(default_graph).await;

        Ok(registry)
    }
}

impl GraphRegistry {
    /// Create a registry backed by in-memory graphs (no database needed).
    pub fn in_memory(default_graph: &str) -> Self {
        Self {
            factory: Box::new(|name: &str| {
                Arc::new(crate::in_memory_graph::InMemoryGraph::new(name)) as Arc<dyn GraphBackend>
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        }
    }

    pub fn default_graph_name(&self) -> &str {
        &self.default_graph
    }

    pub async fn get(&self, graph_name: &str) -> Arc<dyn GraphBackend> {
        let mut cache = self.graphs.lock().await;
        if let Some(existing) = cache.get(graph_name) {
            tracing::debug!(graph = %graph_name, "graph_registry: cache hit");
            return Arc::clone(existing);
        }

        tracing::info!(graph = %graph_name, "graph_registry: creating new graph backend");
        let arc = (self.factory)(graph_name);
        if let Err(e) = arc.setup_schema().await {
            tracing::warn!("Failed to setup schema for graph '{graph_name}': {e}");
        }
        cache.insert(graph_name.to_string(), Arc::clone(&arc));
        arc
    }

    pub async fn get_default(&self) -> Arc<dyn GraphBackend> {
        self.get(&self.default_graph).await
    }

    pub async fn resolve(&self, graph_name: Option<&str>) -> Arc<dyn GraphBackend> {
        match graph_name {
            Some(name) => self.get(name).await,
            None => self.get_default().await,
        }
    }

    pub async fn list(&self) -> Vec<String> {
        let cache = self.graphs.lock().await;
        cache.keys().cloned().collect()
    }

    pub async fn drop_graph(&self, graph_name: &str) -> Result<()> {
        let graph = {
            let mut cache = self.graphs.lock().await;
            cache.remove(graph_name)
        };
        match graph {
            Some(g) => g.drop_and_reinitialise().await,
            None => {
                // Not cached — create a temporary client to drop it
                let g = self.get(graph_name).await;
                let result = g.drop_and_reinitialise().await;
                self.graphs.lock().await.remove(graph_name);
                result
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl GraphClient {
    pub async fn connect(connection_string: &str, graph_name: &str) -> Result<Self> {
        let info: FalkorConnectionInfo = connection_string.try_into().map_err(|e| {
            GraphConnectError::new(format!("invalid FalkorDB connection string: {e}"))
        })?;
        let client = FalkorClientBuilder::new_async()
            .with_connection_info(info)
            .build()
            .await
            .map_err(|e| GraphConnectError::new(format!("failed to connect to FalkorDB: {e}")))?;
        let pool: Vec<Mutex<AsyncGraph>> = (0..DEFAULT_POOL_SIZE)
            .map(|_| Mutex::new(client.select_graph(graph_name)))
            .collect();
        Ok(Self {
            pool: Arc::new(pool),
            next: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            graph_name: graph_name.to_string(),
        })
    }

    pub async fn ping(&self) -> Result<()> {
        let mut graph = self.conn().lock().await;
        graph
            .query("RETURN 1")
            .execute()
            .await
            .map_err(|e| GraphConnectError::new(format!("ping failed: {e}")))?;
        Ok(())
    }

    pub async fn setup_schema(&self) -> Result<()> {
        let indices = vec![
            format!(
                "CREATE FULLTEXT INDEX FOR (e:Entity) ON (e.name, e.content)"
            ),
            format!(
                "CREATE VECTOR INDEX FOR (e:Entity) ON (e.embedding) OPTIONS {{dimension: {EMBEDDING_DIM}, similarityFunction: 'cosine'}}"
            ),
            format!(
                "CREATE VECTOR INDEX FOR ()-[r:RELATION]-() ON (r.embedding) OPTIONS {{dimension: {EMBEDDING_DIM}, similarityFunction: 'cosine'}}"
            ),
            "CREATE INDEX FOR ()-[r:RELATION]-() ON (r.invalid_at)".to_string(),
            "CREATE FULLTEXT INDEX FOR ()-[r:RELATION]-() ON (r.fact)".to_string(),
            "CREATE INDEX FOR ()-[r:RELATION]-() ON (r.last_accessed_at)".to_string(),
            "CREATE INDEX FOR ()-[r:RELATION]-() ON (r.archived)".to_string(),
        ];

        let mut graph = self.conn().lock().await;
        for query in indices {
            match graph.query(&query).execute().await {
                Ok(_) => tracing::info!("Index created"),
                Err(e) if e.to_string().to_lowercase().contains("already") => {
                    tracing::debug!("Index already exists, skipping");
                }
                Err(e) => tracing::warn!("Index creation warning: {e}"),
            }
        }
        Ok(())
    }

    pub async fn drop_and_reinitialise(&self) -> Result<()> {
        {
            let mut graph = self.conn().lock().await;
            graph.delete().await.context("failed to delete graph")?;
            tracing::info!("Graph '{}' dropped", self.graph_name);
        }
        self.setup_schema().await?;
        tracing::info!("Schema recreated for '{}'", self.graph_name);
        Ok(())
    }

    pub async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> {
        // Search each word independently and merge results, so "Alice sister" finds "Alice"
        let tokens: Vec<&str> = query_str
            .split_whitespace()
            .filter(|t| t.len() >= 2)
            .filter(|t| !STOP_WORDS.contains(&t.to_lowercase().as_str()))
            // Strip punctuation that breaks fulltext queries (e.g. &, —, (, ))
            .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
            .take(20)
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();

        let mut graph = self.conn().lock().await;
        for token in tokens {
            // Strip non-alphanumeric chars from edges of token
            let clean: String = token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '\'')
                .collect();
            if clean.is_empty() {
                continue;
            }
            let safe = clean.replace('\'', "\\'");
            let query = format!(
                "CALL db.idx.fulltext.queryNodes('Entity', '{}') YIELD node RETURN node LIMIT 5",
                safe
            );
            match graph.query(&query).execute().await {
                Ok(result) => {
                    let rows: Vec<Vec<FalkorValue>> = result.data.collect();
                    for row in rows {
                        if let Some(entity) = row.into_iter().next().and_then(node_to_entity_row) {
                            match entity {
                                Ok(e) if seen.insert(e.id.clone()) => results.push(Ok(e)),
                                Ok(_) => {}
                                Err(e) => results.push(Err(e)),
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(token = %clean, graph = %self.graph_name, "fulltext search failed for token: {e}");
                    // Skip this token, don't fail the whole search
                }
            }
        }
        results.into_iter().collect()
    }

    pub async fn vector_search_entities(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EntityRow, f32)>> {
        let vec_lit = vec_literal(embedding);
        let query = format!(
            "CALL db.idx.vector.queryNodes('Entity', 'embedding', {k}, {vec_lit}) \
             YIELD node, score \
             RETURN node, score \
             ORDER BY score DESC"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("vector search entities failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 2 {
                    return None;
                }
                let score = extract_float(&row[1]);
                node_to_entity_row(row.into_iter().next()?).map(|e| e.map(|e| (e, score)))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn vector_search_edges_scored(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EdgeRow, f32)>> {
        let vec_lit = vec_literal(embedding);
        let query = format!(
            "CALL db.idx.vector.queryRelationships('RELATION', 'embedding', {k}, {vec_lit}) \
             YIELD relationship, score \
             WHERE score > 0.3 \
               AND relationship.invalid_at IS NULL \
               AND (relationship.archived IS NULL OR relationship.archived = false) \
             MATCH (a)-[relationship]->(b) \
             RETURN relationship, a, b, score \
             ORDER BY score DESC \
             LIMIT {k}"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("vector search edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 4 {
                    return None;
                }
                let mut it = row.into_iter();
                let rel = it.next()?;
                let src = it.next()?;
                let dst = it.next()?;
                let score_val = it.next()?;
                let score = extract_float(&score_val);
                Some(edge_row_from_values(rel, src, dst).map(|e| (e, score)))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn fulltext_search_edges(&self, query_str: &str) -> Result<Vec<EdgeRow>> {
        // FalkorDB queryRelationships returns all edges with equal scores (bug/limitation),
        // so we use CONTAINS on fact text instead — correct and fast for small graphs.
        let tokens: Vec<String> = query_str
            .split_whitespace()
            .filter(|t| t.len() > 2 && !STOP_WORDS.contains(&t.to_lowercase().as_str()))
            .map(|t| t.replace('\'', "\\'").to_lowercase())
            .collect();

        if tokens.is_empty() {
            return Ok(vec![]);
        }

        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();

        let mut graph = self.conn().lock().await;
        for token in &tokens {
            let query = format!(
                "MATCH (a)-[r:RELATION]->(b) \
                 WHERE r.invalid_at IS NULL \
                   AND (r.archived IS NULL OR r.archived = false) \
                   AND toLower(r.fact) CONTAINS '{}' \
                 RETURN r, a, b LIMIT 15",
                token
            );
            let result = graph
                .query(&query)
                .execute()
                .await
                .context("edge contains search failed")?;
            let rows: Vec<Vec<FalkorValue>> = result.data.collect();
            for row in rows {
                let Ok([rel, src, dst]) = take_n(row) else {
                    continue;
                };
                match edge_row_from_values(rel, src, dst) {
                    Ok(e) if seen.insert(e.edge_id) => results.push(e),
                    _ => {}
                }
            }
        }
        Ok(results)
    }

    pub async fn fulltext_search_edges_at(
        &self,
        query_str: &str,
        at: DateTime<Utc>,
    ) -> Result<Vec<EdgeRow>> {
        let at_iso = at.to_rfc3339();

        let tokens: Vec<String> = query_str
            .split_whitespace()
            .filter(|t| t.len() > 2 && !STOP_WORDS.contains(&t.to_lowercase().as_str()))
            .map(|t| t.replace('\'', "\\'").to_lowercase())
            .collect();

        if tokens.is_empty() {
            return Ok(vec![]);
        }

        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();

        let mut graph = self.conn().lock().await;
        for token in &tokens {
            let query = format!(
                "MATCH (a)-[r:RELATION]->(b) \
                 WHERE toLower(r.fact) CONTAINS '{token}' \
                 AND r.valid_at <= '{at_iso}' \
                 AND (r.invalid_at IS NULL OR r.invalid_at > '{at_iso}') \
                 RETURN r, a, b LIMIT 15"
            );
            let result = graph
                .query(&query)
                .execute()
                .await
                .context("edge contains search (temporal) failed")?;
            let rows: Vec<Vec<FalkorValue>> = result.data.collect();
            for row in rows {
                let Ok([rel, src, dst]) = take_n(row) else {
                    continue;
                };
                match edge_row_from_values(rel, src, dst) {
                    Ok(e) if seen.insert(e.edge_id) => results.push(e),
                    _ => {}
                }
            }
        }
        Ok(results)
    }

    pub async fn vector_search_edges_at(
        &self,
        embedding: &[f32],
        k: usize,
        at: DateTime<Utc>,
    ) -> Result<Vec<EdgeRow>> {
        let at_iso = at.to_rfc3339();
        let vec_lit = vec_literal(embedding);
        let query = format!(
            "CALL db.idx.vector.queryRelationships('RELATION', 'embedding', {k}, {vec_lit}) \
             YIELD relationship, score \
             WHERE relationship.valid_at <= '{at_iso}' \
             AND (relationship.invalid_at IS NULL OR relationship.invalid_at > '{at_iso}') \
             MATCH (a)-[relationship]->(b) \
             RETURN relationship, a, b, score \
             ORDER BY score DESC"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("vector search edges (temporal) failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 4 {
                    return None;
                }
                let mut it = row.into_iter();
                let rel = it.next()?;
                let src = it.next()?;
                let dst = it.next()?;
                let _score = it.next()?;
                Some(edge_row_from_values(rel, src, dst))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn walk_one_hop_at(
        &self,
        entity_ids: &[String],
        limit: usize,
        at: DateTime<Utc>,
    ) -> Result<Vec<EdgeRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let at_iso = at.to_rfc3339();
        let ids_list = entity_ids
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "MATCH (e:Entity)-[r:RELATION]-(n:Entity) \
             WHERE e.id IN [{ids_list}] \
             AND r.valid_at <= '{at_iso}' \
             AND (r.invalid_at IS NULL OR r.invalid_at > '{at_iso}') \
             RETURN r, e, n \
             ORDER BY r.salience DESC, r.valid_at DESC \
             LIMIT {limit}"
        );
        self.query_edge_rows(&query).await
    }
    pub async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> {
        let node_id = sanitise(node_id);
        let query = format!(
            "MATCH (a:Entity {{id: '{node_id}'}})-[r:RELATION]->(b:Entity) \
             WHERE r.invalid_at IS NULL \
               AND (r.archived IS NULL OR r.archived = false) \
             RETURN r, a, b"
        );
        self.query_edge_rows(&query).await
    }

    async fn query_edge_rows(&self, query: &str) -> Result<Vec<EdgeRow>> {
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(query)
            .execute()
            .await
            .context("edge query failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                let [rel, src, dst] = take_n(row).ok()?;
                Some(edge_row_from_values(rel, src, dst))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn upsert_entity(&self, entity: &crate::models::Entity) -> Result<()> {
        let vec_lit = vec_literal(&entity.embedding);
        let hint = entity.hint.as_deref().unwrap_or("");
        let content_clause = match &entity.content {
            Some(c) => format!(", e.content = '{}'", c.replace('\'', "\\'")),
            None => String::new(),
        };
        let query = format!(
            "MERGE (e:Entity {{id: '{}'}}) \
             SET e.name = '{}', e.entity_type = '{}', \
                 e.resolved = {}, e.hint = '{}', e.created_at = '{}', \
                 e.embedding = {}{} \
             RETURN e.id",
            entity.id,
            entity.name.replace('\'', "\\'"),
            entity.entity_type,
            entity.resolved,
            hint.replace('\'', "\\'"),
            entity.created_at.to_rfc3339(),
            vec_lit,
            content_clause,
        );
        let mut graph = self.conn().lock().await;
        graph
            .query(&query)
            .execute()
            .await
            .context("upsert entity failed")?;
        Ok(())
    }

    pub async fn create_edge(
        &self,
        from_id: &str,
        to_id: &str,
        rel: &crate::models::Relation,
    ) -> Result<i64> {
        let vec_lit = vec_literal(&rel.embedding);
        let invalid_at = match &rel.invalid_at {
            Some(t) => format!("'{}'", t.to_rfc3339()),
            None => "null".to_string(),
        };
        let expires_at = match &rel.expires_at {
            Some(t) => format!("'{}'", t.to_rfc3339()),
            None => "null".to_string(),
        };
        let query = format!(
            "MATCH (a:Entity {{id: '{from_id}'}}) \
             MATCH (b:Entity {{id: '{to_id}'}}) \
             CREATE (a)-[r:RELATION {{ \
                 fact: '{}', relation_type: '{}', embedding: {vec_lit}, \
                 source_agents: '{}', \
                 valid_at: '{}', invalid_at: {invalid_at}, \
                 confidence: {}, salience: 0, created_at: '{}', \
                 last_accessed_at: '{}', decayed_confidence: {}, \
                 memory_tier: '{}', expires_at: {expires_at} \
             }}]->(b) \
             RETURN id(r)",
            rel.fact.replace('\'', "\\'"),
            rel.relation_type,
            rel.source_agents
                .iter()
                .map(|s| sanitise(s))
                .collect::<Vec<_>>()
                .join("|"),
            rel.valid_at.to_rfc3339(),
            rel.confidence,
            rel.created_at.to_rfc3339(),
            rel.created_at.to_rfc3339(),
            rel.confidence,
            rel.memory_tier,
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("create edge failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            if let Some(v) = row.into_iter().next() {
                return Ok(extract_int(&v));
            }
        }
        Ok(-1)
    }

    pub async fn promote_working_memory(&self) -> Result<usize> {
        let one_hour_ago_rfc = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let one_hour_ago = falkor_strip_tz(&one_hour_ago_rfc);
        let query = format!(
            "\
            MATCH ()-[r:RELATION]-() \
            WHERE r.invalid_at IS NULL \
              AND (r.archived IS NULL OR r.archived = false) \
              AND r.memory_tier = 'working' \
              AND ( \
                r.salience >= 3 \
                OR ( \
                  substring(r.created_at, 0, 19) < '{one_hour_ago}' \
                  AND r.confidence > 0.7 \
                ) \
              ) \
            SET r.memory_tier = 'long_term' \
            RETURN count(r) AS promoted"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(query)
            .execute()
            .await
            .context("promote working memory failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            if let Some(v) = row.into_iter().next() {
                return Ok(extract_int(&v) as usize);
            }
        }
        Ok(0)
    }

    pub async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> {
        let now_rfc = now.to_rfc3339();
        let now_iso = falkor_strip_tz(&now_rfc);
        let query = format!(
            "MATCH ()-[r:RELATION]-() \
             WHERE r.invalid_at IS NULL \
               AND (r.archived IS NULL OR r.archived = false) \
               AND r.expires_at IS NOT NULL \
               AND substring(r.expires_at, 0, 19) <= '{now_iso}' \
             SET r.invalid_at = '{}' \
             RETURN count(r) AS expired",
            now.to_rfc3339()
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("expire TTL edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            if let Some(v) = row.into_iter().next() {
                return Ok(extract_int(&v) as usize);
            }
        }
        Ok(0)
    }

    pub async fn memory_tier_stats(&self) -> Result<crate::models::MemoryTierStats> {
        let query = "\
            MATCH ()-[r:RELATION]-() \
            WHERE r.invalid_at IS NULL \
              AND (r.archived IS NULL OR r.archived = false) \
            RETURN \
              sum(CASE WHEN r.memory_tier = 'working' THEN 1 ELSE 0 END) AS working, \
              sum(CASE WHEN r.memory_tier = 'long_term' OR r.memory_tier IS NULL THEN 1 ELSE 0 END) AS long_term";
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(query)
            .execute()
            .await
            .context("memory tier stats failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            let mut iter = row.into_iter();
            let working_count = iter.next().map(|v| extract_int(&v) as usize).unwrap_or(0);
            let long_term_count = iter.next().map(|v| extract_int(&v) as usize).unwrap_or(0);
            return Ok(crate::models::MemoryTierStats {
                working_count,
                long_term_count,
            });
        }
        Ok(crate::models::MemoryTierStats {
            working_count: 0,
            long_term_count: 0,
        })
    }

    pub async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        let query = format!(
            "MATCH ()-[r:RELATION]->() WHERE id(r) = {edge_id} \
             SET r.invalid_at = '{}'",
            at.to_rfc3339()
        );
        let mut graph = self.conn().lock().await;
        graph
            .query(&query)
            .execute()
            .await
            .context("invalidate edge failed")?;
        Ok(())
    }

    pub async fn compound_edge_confidence(
        &self,
        edge_id: i64,
        new_agent: &str,
        new_confidence: f32,
    ) -> Result<f32> {
        let safe_agent = sanitise(new_agent);
        // Bayesian combination: 1 - (1 - existing) * (1 - new), capped at 0.99
        let query = format!(
            "MATCH ()-[r:RELATION]-() WHERE id(r) = {edge_id} \
             SET r.confidence = CASE \
               WHEN (1.0 - (1.0 - r.confidence) * (1.0 - {new_confidence})) > 0.99 THEN 0.99 \
               ELSE (1.0 - (1.0 - r.confidence) * (1.0 - {new_confidence})) \
             END, \
             r.decayed_confidence = CASE \
               WHEN (1.0 - (1.0 - r.decayed_confidence) * (1.0 - {new_confidence})) > 0.99 THEN 0.99 \
               ELSE (1.0 - (1.0 - r.decayed_confidence) * (1.0 - {new_confidence})) \
             END, \
             r.source_agents = CASE \
               WHEN r.source_agents CONTAINS '{safe_agent}' THEN r.source_agents \
               ELSE r.source_agents + '|{safe_agent}' \
             END \
             RETURN r.confidence"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("compound edge confidence failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let conf = rows
            .into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .map(|v| extract_float(&v))
            .unwrap_or(0.0);
        Ok(conf)
    }

    pub async fn walk_one_hop(&self, entity_ids: &[String], limit: usize) -> Result<Vec<EdgeRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        let ids_list = entity_ids
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "MATCH (e:Entity)-[r:RELATION]-(n:Entity) \
             WHERE e.id IN [{ids_list}] AND r.invalid_at IS NULL \
               AND (r.archived IS NULL OR r.archived = false) \
             RETURN r, e, n \
             ORDER BY r.salience DESC, r.valid_at DESC \
             LIMIT {limit}"
        );
        self.query_edge_rows(&query).await
    }

    pub async fn walk_n_hops(
        &self,
        seed_entity_ids: &[String],
        max_hops: usize,
        limit_per_hop: usize,
    ) -> Result<Vec<(EdgeRow, usize)>> {
        let mut all_edges: Vec<(EdgeRow, usize)> = Vec::new();
        let mut seen_edge_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut frontier: Vec<String> = seed_entity_ids.to_vec();

        for hop in 1..=max_hops {
            if frontier.is_empty() {
                break;
            }
            let hop_edges = self.walk_one_hop(&frontier, limit_per_hop).await?;
            if hop_edges.is_empty() {
                break;
            }

            let mut next_frontier = std::collections::HashSet::new();
            for edge in hop_edges {
                if seen_edge_ids.insert(edge.edge_id) {
                    next_frontier.insert(edge.subject_id.clone());
                    next_frontier.insert(edge.object_id.clone());
                    all_edges.push((edge, hop));
                }
            }

            // Remove seed entities so we only expand outward
            for id in &frontier {
                next_frontier.remove(id);
            }
            frontier = next_frontier.into_iter().collect();
        }

        Ok(all_edges)
    }
    pub async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<usize> {
        let stale_before_iso = stale_before.to_rfc3339();
        let now_rfc = now.to_rfc3339();
        let now_no_tz = falkor_strip_tz(&now_rfc);
        let days_stale_expr = falkor_approx_days_clause("accessed_dt", "now_dt", "days_stale");
        let query = format!(
            "MATCH ()-[r:RELATION]-() \
             WHERE r.invalid_at IS NULL \
               AND r.last_accessed_at < '{stale_before_iso}' \
             WITH r, \
               localdatetime(substring(r.last_accessed_at, 0, 19)) AS accessed_dt, \
               localdatetime('{now_no_tz}') AS now_dt \
             WITH r, \
               {days_stale_expr} \
             SET r.decayed_confidence = CASE \
               WHEN days_stale > 30 THEN r.confidence * (0.995 ^ (days_stale - 30)) \
               ELSE r.confidence \
             END, \
             r.salience = CASE \
               WHEN r.salience > 0 THEN r.salience - 1 \
               ELSE 0 \
             END \
             RETURN count(r) AS updated"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("decay stale edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let count = rows
            .into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .map(|v| extract_int(&v) as usize)
            .unwrap_or(0);
        Ok(count)
    }

    pub async fn find_close_unlinked(
        &self,
        node_id: &str,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Vec<(EntityRow, f32)>> {
        let vec_lit = vec_literal(embedding);
        let safe_node_id = sanitise(node_id);
        let query = format!(
            "CALL db.idx.vector.queryNodes('Entity', 'embedding', 20, {vec_lit}) \
             YIELD node AS b, score \
             WHERE b.id <> '{safe_node_id}' AND score > {threshold} \
               AND NOT (b)-[:RELATION]-(:Entity {{id: '{safe_node_id}'}}) \
             RETURN b, score \
             ORDER BY score DESC \
             LIMIT 10"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("find close unlinked failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 2 {
                    return None;
                }
                let score = extract_float(&row[1]);
                node_to_entity_row(row.into_iter().next()?).map(|e| e.map(|e| (e, score)))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn find_placeholder_nodes(&self, cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> {
        let query = format!(
            "MATCH (e:Entity) \
             WHERE e.resolved = false AND e.created_at < '{}' \
             RETURN e \
             LIMIT 20",
            cutoff.to_rfc3339()
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("find placeholders failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| row.into_iter().next().and_then(node_to_entity_row))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> {
        let placeholder_id = sanitise(placeholder_id);
        let resolved_id = sanitise(resolved_id);
        let mut graph = self.conn().lock().await;

        let q1 = format!(
            "MATCH (p:Entity {{id: '{placeholder_id}'}})-[r:RELATION]->(o:Entity) \
             MATCH (res:Entity {{id: '{resolved_id}'}}) \
             CREATE (res)-[:RELATION {{ \
                 fact: r.fact, relation_type: r.relation_type, \
                 embedding: r.embedding, \
                 source_agents: r.source_agents, \
                 valid_at: r.valid_at, invalid_at: r.invalid_at, \
                 confidence: r.confidence, salience: r.salience, created_at: r.created_at, \
                 last_accessed_at: r.last_accessed_at, decayed_confidence: r.decayed_confidence \
             }}]->(o) DELETE r"
        );
        graph
            .query(&q1)
            .execute()
            .await
            .context("merge placeholder step 1 failed")?;

        let q2 = format!(
            "MATCH (o:Entity)-[r:RELATION]->(p:Entity {{id: '{placeholder_id}'}}) \
             MATCH (res:Entity {{id: '{resolved_id}'}}) \
             CREATE (o)-[:RELATION {{ \
                 fact: r.fact, relation_type: r.relation_type, \
                 embedding: r.embedding, \
                 source_agents: r.source_agents, \
                 valid_at: r.valid_at, invalid_at: r.invalid_at, \
                 confidence: r.confidence, salience: r.salience, created_at: r.created_at, \
                 last_accessed_at: r.last_accessed_at, decayed_confidence: r.decayed_confidence \
             }}]->(res) DELETE r"
        );
        graph
            .query(&q2)
            .execute()
            .await
            .context("merge placeholder step 2 failed")?;

        let q3 = format!("MATCH (p:Entity {{id: '{placeholder_id}'}}) DELETE p");
        graph
            .query(&q3)
            .execute()
            .await
            .context("merge placeholder step 3 failed")?;

        Ok(())
    }
    /// Return graph statistics: (entity_count, fact_count, oldest_valid_at, newest_valid_at, avg_confidence).
    pub async fn graph_stats(&self) -> Result<crate::models::GraphStats> {
        let mut graph = self.conn().lock().await;

        // Entity count
        let result = graph
            .query("MATCH (e:Entity) RETURN count(e) AS entity_count")
            .execute()
            .await
            .context("graph_stats entity count failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let entity_count = rows
            .into_iter()
            .next()
            .and_then(|r| r.into_iter().next())
            .map(|v| extract_int(&v) as usize)
            .unwrap_or(0);

        // Fact stats
        let result = graph.query(
            "MATCH ()-[r:RELATION]-() WHERE r.invalid_at IS NULL \
             RETURN count(r) AS fact_count, min(r.valid_at) AS oldest, max(r.valid_at) AS newest, avg(r.confidence) AS avg_conf"
        ).execute().await.context("graph_stats fact stats failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            let vals: Vec<FalkorValue> = row;
            let edge_count = vals.first().map(extract_int).unwrap_or(0) as usize;
            let oldest_valid_at = vals.get(1).and_then(|v| {
                let s = extract_string(v);
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            });
            let newest_valid_at = vals.get(2).and_then(|v| {
                let s = extract_string(v);
                if s.is_empty() {
                    None
                } else {
                    Some(s)
                }
            });
            let avg_confidence = vals.get(3).map(extract_float).unwrap_or(0.0);
            Ok(crate::models::GraphStats {
                entity_count,
                edge_count,
                oldest_valid_at,
                newest_valid_at,
                avg_confidence,
            })
        } else {
            Ok(crate::models::GraphStats {
                entity_count,
                edge_count: 0,
                oldest_valid_at: None,
                newest_valid_at: None,
                avg_confidence: 0.0,
            })
        }
    }
    pub async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        let query = "MATCH (e:Entity) RETURN e ORDER BY e.name".to_string();
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("dump entities failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| row.into_iter().next().and_then(node_to_entity_row))
            .collect()
    }

    pub async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        let query = "MATCH (a)-[r:RELATION]->(b) RETURN r, a, b ORDER BY a.name".to_string();
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("dump edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let mut out = Vec::new();
        for row in rows {
            let Ok([rel, src, dst]) = take_n(row) else {
                continue;
            };
            if let Ok(e) = edge_row_from_values(rel, src, dst) {
                out.push(e);
            }
        }
        Ok(out)
    }

    pub async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        let entity_id = sanitise(entity_id);
        let query = format!(
            "MATCH (a:Entity {{id: '{entity_id}'}})-[r:RELATION]-(b:Entity) \
             WHERE r.invalid_at IS NULL \
               AND (r.archived IS NULL OR r.archived = false) \
             RETURN r.fact \
             LIMIT 20"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("get entity facts failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let facts = rows
            .into_iter()
            .filter_map(|row| row.into_iter().next())
            .map(|v| extract_string(&v))
            .collect();
        Ok(facts)
    }

    pub async fn create_supersession(
        &self,
        old_edge_id: i64,
        new_edge_id: i64,
        superseded_at: DateTime<Utc>,
        old_fact: &str,
        new_fact: &str,
    ) -> Result<()> {
        let safe_old_fact = sanitise(old_fact);
        let safe_new_fact = sanitise(new_fact);
        let query = format!(
            "CREATE (:Supersession {{ \
                old_edge_id: {old_edge_id}, \
                new_edge_id: {new_edge_id}, \
                superseded_at: '{}', \
                old_fact: '{safe_old_fact}', \
                new_fact: '{safe_new_fact}' \
            }})",
            superseded_at.to_rfc3339(),
        );
        let mut graph = self.conn().lock().await;
        graph
            .query(&query)
            .execute()
            .await
            .context("create supersession failed")?;
        Ok(())
    }

    pub async fn get_supersession_chain(&self, edge_id: i64) -> Result<Vec<SupersessionRecord>> {
        let query = format!(
            "MATCH (s:Supersession) \
             WHERE s.old_edge_id = {edge_id} OR s.new_edge_id = {edge_id} \
             RETURN s.old_edge_id, s.new_edge_id, s.superseded_at, s.old_fact, s.new_fact \
             ORDER BY s.superseded_at"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("get supersession chain failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter().map(supersession_from_row).collect()
    }

    pub async fn get_provenance(&self, edge_id: i64) -> Result<crate::models::ProvenanceResponse> {
        let chain = self.get_supersession_chain(edge_id).await?;
        let superseded_by = chain.iter().find(|s| s.old_edge_id == edge_id).cloned();
        let supersedes: Vec<SupersessionRecord> = chain
            .into_iter()
            .filter(|s| s.new_edge_id == edge_id)
            .collect();
        Ok(crate::models::ProvenanceResponse {
            edge_id,
            superseded_by,
            supersedes,
        })
    }
    pub async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        let entity_id = sanitise(entity_id);
        let query = format!("MATCH (e:Entity {{id: '{entity_id}'}}) RETURN e");
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("get entity by id failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        Ok(rows
            .into_iter()
            .filter_map(|row| row.into_iter().next().and_then(node_to_entity_row))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .next())
    }

    pub async fn save_source_credibility(
        &self,
        cred: &crate::credibility::SourceCredibility,
    ) -> Result<()> {
        let agent_id = sanitise(&cred.agent_id);
        let updated_at = chrono::Utc::now().to_rfc3339();
        let query = format!(
            "MERGE (s:SourceCredibility {{agent_id: '{agent_id}'}}) \
             SET s.credibility = {}, \
                 s.fact_count = {}, \
                 s.contradiction_rate = {}, \
                 s.updated_at = '{updated_at}'",
            cred.credibility, cred.fact_count, cred.contradiction_rate,
        );
        let mut graph = self.conn().lock().await;
        graph
            .query(&query)
            .execute()
            .await
            .context("save source credibility failed")?;
        Ok(())
    }

    pub async fn load_all_source_credibility(
        &self,
    ) -> Result<Vec<crate::credibility::SourceCredibility>> {
        let query = "MATCH (s:SourceCredibility) \
                     RETURN s.agent_id, s.credibility, s.fact_count, s.contradiction_rate";
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(query)
            .execute()
            .await
            .context("load source credibility failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let mut out = Vec::new();
        for row in rows {
            let Ok([a, b, c, d]) = take_n(row) else {
                continue;
            };
            let agent_id = extract_string(&a);
            let credibility = extract_float(&b);
            let fact_count = extract_int(&c) as usize;
            let contradiction_rate = extract_float(&d);
            out.push(crate::credibility::SourceCredibility {
                agent_id,
                credibility,
                fact_count,
                contradiction_rate,
            });
        }
        Ok(out)
    }

    pub async fn delete_entity(&self, entity_id: &str) -> Result<usize> {
        let entity_id = sanitise(entity_id);
        let now = Utc::now();

        // Invalidate all edges connected to this entity (both directions)
        let invalidate_query = format!(
            "MATCH (e:Entity {{id: '{entity_id}'}})-[r:RELATION]-() SET r.invalid_at = '{}' RETURN count(r)",
            now.to_rfc3339()
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&invalidate_query)
            .execute()
            .await
            .context("delete_entity: invalidate edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let count = rows
            .into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .map(|v| extract_int(&v) as usize)
            .unwrap_or(0);

        // Delete the entity node
        let delete_query = format!("MATCH (e:Entity {{id: '{entity_id}'}}) DELETE e");
        graph
            .query(&delete_query)
            .execute()
            .await
            .context("delete_entity: delete node failed")?;

        Ok(count)
    }

    pub async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        let entity_id = sanitise(entity_id);
        let new_name = sanitise(new_name);
        let query = format!("MATCH (e:Entity {{id: '{entity_id}'}}) SET e.name = '{new_name}'");
        let mut graph = self.conn().lock().await;
        graph
            .query(&query)
            .execute()
            .await
            .context("rename_entity failed")?;
        Ok(())
    }

    pub async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> {
        let entity_id = sanitise(entity_id);
        let key = sanitise(key);
        // Store booleans as native booleans, not strings
        let cypher_value = match value {
            "true" => "true".to_string(),
            "false" => "false".to_string(),
            _ => format!("'{}'", sanitise(value)),
        };
        let query =
            format!("MATCH (e:Entity {{id: '{entity_id}'}}) SET e.`{key}` = {cypher_value}");
        let mut graph = self.conn().lock().await;
        graph
            .query(&query)
            .execute()
            .await
            .context("set_entity_property failed")?;
        Ok(())
    }

    pub async fn list_entities_by_recency(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EntityRow>> {
        let query = format!(
            "MATCH (e:Entity) RETURN e ORDER BY e.created_at DESC SKIP {offset} LIMIT {limit}"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("list_entities_by_recency failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| row.into_iter().next())
            .filter_map(node_to_entity_row)
            .collect::<Result<Vec<_>>>()
    }

    pub async fn find_entity_by_property(
        &self,
        key: &str,
        value: &str,
    ) -> Result<Option<EntityRow>> {
        let key = sanitise(key);
        let value = sanitise(value);
        // Match both string and boolean forms (e.g. is_principal = 'true' OR is_principal = true)
        let query = format!(
            "MATCH (e:Entity) WHERE e.`{key}` = '{value}' OR e.`{key}` = {value} RETURN e LIMIT 1"
        );
        let mut graph = self.conn().lock().await;
        let result = graph
            .query(&query)
            .execute()
            .await
            .context("find_entity_by_property failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        for row in rows {
            if let Some(val) = row.into_iter().next() {
                if let Some(Ok(entity)) = node_to_entity_row(val) {
                    return Ok(Some(entity));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl GraphBackend for GraphClient {
    fn graph_name(&self) -> &str {
        &self.graph_name
    }
    async fn ping(&self) -> Result<()> {
        self.ping().await
    }
    async fn setup_schema(&self) -> Result<()> {
        self.setup_schema().await
    }
    async fn drop_and_reinitialise(&self) -> Result<()> {
        self.drop_and_reinitialise().await
    }
    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> {
        self.fulltext_search_entities(query_str).await
    }
    async fn vector_search_entities(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EntityRow, f32)>> {
        self.vector_search_entities(embedding, k).await
    }
    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        self.get_entity_by_id(entity_id).await
    }
    async fn fulltext_search_edges(
        &self,
        query_str: &str,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
        match at {
            Some(t) => self.fulltext_search_edges_at(query_str, t).await,
            None => GraphClient::fulltext_search_edges(self, query_str).await,
        }
    }
    async fn vector_search_edges_scored(
        &self,
        embedding: &[f32],
        k: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, f32)>> {
        match at {
            Some(t) => {
                let rows = self.vector_search_edges_at(embedding, k, t).await?;
                Ok(rows
                    .into_iter()
                    .map(|r| {
                        let score = cosine_sim(embedding, &r.embedding);
                        (r, score)
                    })
                    .collect())
            }
            None => GraphClient::vector_search_edges_scored(self, embedding, k).await,
        }
    }
    async fn walk_n_hops(
        &self,
        seed_entity_ids: &[String],
        max_hops: usize,
        limit_per_hop: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, usize)>> {
        // Delegate to inherent walk_n_hops which uses walk_one_hop or walk_one_hop_at internally
        match at {
            Some(t) => {
                // Manually do n-hop walk with temporal filtering
                let mut results = Vec::new();
                let mut frontier: Vec<String> = seed_entity_ids.to_vec();
                let mut visited_edges: std::collections::HashSet<i64> =
                    std::collections::HashSet::new();
                for hop in 1..=max_hops {
                    let hop_edges = self.walk_one_hop_at(&frontier, limit_per_hop, t).await?;
                    let mut next_frontier = Vec::new();
                    for edge in hop_edges {
                        if visited_edges.insert(edge.edge_id) {
                            if !frontier.contains(&edge.subject_id) {
                                next_frontier.push(edge.subject_id.clone());
                            }
                            if !frontier.contains(&edge.object_id) {
                                next_frontier.push(edge.object_id.clone());
                            }
                            results.push((edge, hop));
                        }
                    }
                    if next_frontier.is_empty() {
                        break;
                    }
                    frontier = next_frontier;
                }
                Ok(results)
            }
            None => GraphClient::walk_n_hops(self, seed_entity_ids, max_hops, limit_per_hop).await,
        }
    }
    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> {
        self.find_all_active_edges_from(node_id).await
    }
    async fn upsert_entity(&self, entity: &crate::models::Entity) -> Result<()> {
        self.upsert_entity(entity).await
    }
    async fn create_edge(
        &self,
        from_id: &str,
        to_id: &str,
        rel: &crate::models::Relation,
    ) -> Result<i64> {
        self.create_edge(from_id, to_id, rel).await
    }
    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        self.invalidate_edge(edge_id, at).await
    }
    async fn delete_entity(&self, entity_id: &str) -> Result<usize> {
        self.delete_entity(entity_id).await
    }
    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> {
        self.merge_placeholder(placeholder_id, resolved_id).await
    }
    async fn promote_working_memory(&self) -> Result<usize> {
        self.promote_working_memory().await
    }
    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> {
        self.expire_ttl_edges(now).await
    }
    async fn memory_tier_stats(&self) -> Result<crate::models::MemoryTierStats> {
        self.memory_tier_stats().await
    }
    async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<usize> {
        self.decay_stale_edges(stale_before, now).await
    }
    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        self.get_entity_facts(entity_id).await
    }
    async fn graph_stats(&self) -> Result<crate::models::GraphStats> {
        self.graph_stats().await
    }
    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        self.dump_all_entities().await
    }
    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        self.dump_all_edges().await
    }
    async fn list_entities_by_recency(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EntityRow>> {
        self.list_entities_by_recency(offset, limit).await
    }
    async fn get_provenance(&self, edge_id: i64) -> Result<crate::models::ProvenanceResponse> {
        self.get_provenance(edge_id).await
    }
    async fn find_close_unlinked(
        &self,
        node_id: &str,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Vec<(EntityRow, f32)>> {
        self.find_close_unlinked(node_id, embedding, threshold)
            .await
    }
    async fn find_placeholder_nodes(&self, cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> {
        self.find_placeholder_nodes(cutoff).await
    }
    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        self.rename_entity(entity_id, new_name).await
    }
    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> {
        self.set_entity_property(entity_id, key, value).await
    }
    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> {
        self.find_entity_by_property(key, value).await
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitise(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

// --- FalkorDB datetime helpers ---
//
// FalkorDB's Cypher dialect does not support `datetime()` arithmetic or
// `duration.between()`.  These helpers paper over the gaps by working
// with ISO-8601 strings and Cypher's `localdatetime()` constructor.

/// Strip the timezone suffix from an RFC 3339 timestamp so it can be
/// used with FalkorDB's `localdatetime()` or plain string comparison.
/// Returns the first 19 characters: `YYYY-MM-DDTHH:MM:SS`.
#[cfg(not(target_arch = "wasm32"))]
fn falkor_strip_tz(rfc3339: &str) -> &str {
    &rfc3339[..19]
}

/// Build a Cypher `WITH` clause that computes an approximate number of
/// days between two `localdatetime` expressions.
///
/// FalkorDB lacks `duration.between()`, so we approximate using year,
/// month, and day components.  The result is bound to `{alias}`.
#[cfg(not(target_arch = "wasm32"))]
fn falkor_approx_days_clause(accessed_expr: &str, now_expr: &str, alias: &str) -> String {
    format!(
        "toFloat(({now_expr}.year - {accessed_expr}.year) * 365 \
              + ({now_expr}.month - {accessed_expr}.month) * 30 \
              + ({now_expr}.day - {accessed_expr}.day)) AS {alias}"
    )
}

// --- Value parsing helpers ---

#[cfg(not(target_arch = "wasm32"))]
fn node_to_entity_row(v: FalkorValue) -> Option<Result<EntityRow>> {
    match v {
        FalkorValue::Node(node) => {
            let p = &node.properties;
            Some(Ok(EntityRow {
                id: prop_string(p, "id"),
                name: prop_string(p, "name"),
                entity_type: prop_string(p, "entity_type"),
                resolved: prop_bool(p, "resolved"),
                hint: prop_opt_string(p, "hint"),
                content: prop_opt_string(p, "content"),
                created_at: prop_string(p, "created_at"),
                embedding: prop_embedding(p, "embedding"),
            }))
        }
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn edge_row_from_values(rel: FalkorValue, src: FalkorValue, dst: FalkorValue) -> Result<EdgeRow> {
    let (
        edge_id,
        fact,
        relation_type,
        confidence,
        salience,
        valid_at,
        invalid_at,
        embedding,
        decayed_confidence,
        source_agents,
        memory_tier,
        expires_at,
    ) = match &rel {
        FalkorValue::Edge(edge) => {
            let p = &edge.properties;
            let confidence = prop_float(p, "confidence");
            let valid_at = prop_string(p, "valid_at");
            let decayed_confidence = {
                let v = prop_float(p, "decayed_confidence");
                if v == 0.0 {
                    confidence
                } else {
                    v
                }
            };
            let source_agents = prop_opt_string(p, "source_agents").unwrap_or_default();
            let memory_tier =
                prop_opt_string(p, "memory_tier").unwrap_or_else(|| "long_term".to_string());
            let expires_at = prop_opt_string(p, "expires_at");
            (
                edge.entity_id,
                prop_string(p, "fact"),
                prop_string(p, "relation_type"),
                confidence,
                prop_int(p, "salience"),
                valid_at,
                prop_opt_string(p, "invalid_at"),
                prop_embedding(p, "embedding"),
                decayed_confidence,
                source_agents,
                memory_tier,
                expires_at,
            )
        }
        _ => anyhow::bail!("expected Edge value"),
    };

    let (subject_id, subject_name) = match &src {
        FalkorValue::Node(node) => {
            let p = &node.properties;
            (prop_string(p, "id"), prop_string(p, "name"))
        }
        _ => (String::new(), String::new()),
    };

    let (object_id, object_name) = match &dst {
        FalkorValue::Node(node) => {
            let p = &node.properties;
            (prop_string(p, "id"), prop_string(p, "name"))
        }
        _ => (String::new(), String::new()),
    };

    Ok(EdgeRow {
        edge_id,
        subject_id,
        subject_name,
        fact,
        relation_type,
        confidence,
        salience,
        valid_at,
        invalid_at,
        object_id,
        object_name,
        embedding,
        decayed_confidence,
        source_agents,
        memory_tier,
        expires_at,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn prop_string(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> String {
    p.get(key).map(extract_string).unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn prop_opt_string(
    p: &std::collections::HashMap<String, FalkorValue>,
    key: &str,
) -> Option<String> {
    p.get(key).and_then(|v| match v {
        FalkorValue::None => None,
        FalkorValue::String(s) if s.is_empty() => None,
        other => Some(extract_string(other)),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn prop_bool(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> bool {
    p.get(key).map(extract_bool).unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn prop_float(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> f32 {
    p.get(key).map(extract_float).unwrap_or(0.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn prop_int(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> i64 {
    p.get(key).map(extract_int).unwrap_or(0)
}

#[cfg(not(target_arch = "wasm32"))]
fn prop_embedding(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> Vec<f32> {
    p.get(key)
        .map(extract_embedding)
        .unwrap_or_else(|| vec![0.0; EMBEDDING_DIM])
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_string(v: &FalkorValue) -> String {
    match v {
        FalkorValue::String(s) => s.clone(),
        FalkorValue::I64(i) => i.to_string(),
        FalkorValue::F64(f) => f.to_string(),
        FalkorValue::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_bool(v: &FalkorValue) -> bool {
    match v {
        FalkorValue::Bool(b) => *b,
        FalkorValue::String(s) => s == "true",
        FalkorValue::I64(i) => *i != 0,
        _ => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_int(v: &FalkorValue) -> i64 {
    match v {
        FalkorValue::I64(i) => *i,
        FalkorValue::F64(f) => *f as i64,
        _ => 0,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_float(v: &FalkorValue) -> f32 {
    match v {
        FalkorValue::F64(f) => *f as f32,
        FalkorValue::I64(i) => *i as f32,
        _ => 0.0,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn extract_embedding(v: &FalkorValue) -> Vec<f32> {
    match v {
        FalkorValue::Array(arr) => arr.iter().map(extract_float).collect(),
        FalkorValue::Vec32(v32) => v32.values.clone(),
        _ => vec![0.0; EMBEDDING_DIM],
    }
}

/// Destructure a row into a fixed-size array, bailing if too short.
#[cfg(not(target_arch = "wasm32"))]
fn take_n<const N: usize>(row: Vec<FalkorValue>) -> Result<[FalkorValue; N]> {
    row.try_into()
        .map_err(|v: Vec<FalkorValue>| anyhow::anyhow!("expected {N} columns, got {}", v.len()))
}

#[cfg(not(target_arch = "wasm32"))]
fn supersession_from_row(row: Vec<FalkorValue>) -> Result<SupersessionRecord> {
    let [a, b, c, d, e] = take_n(row)?;
    let old_edge_id = extract_int(&a);
    let new_edge_id = extract_int(&b);
    let superseded_at_str = extract_string(&c);
    let old_fact = extract_string(&d);
    let new_fact = extract_string(&e);
    let superseded_at = superseded_at_str
        .parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    Ok(SupersessionRecord {
        old_edge_id,
        new_edge_id,
        superseded_at,
        old_fact,
        new_fact,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn vec_literal(v: &[f32]) -> String {
    let inner = v
        .iter()
        .map(|f| format!("{:.6}", f))
        .collect::<Vec<_>>()
        .join(", ");
    format!("vecf32([{inner}])")
}
