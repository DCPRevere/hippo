use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use falkordb::{AsyncGraph, FalkorAsyncClient, FalkorClientBuilder, FalkorConnectionInfo, FalkorValue};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;

static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
        "have", "has", "had", "do", "does", "did", "will", "would", "could",
        "should", "may", "might", "shall", "can", "my", "your", "his", "her",
        "its", "our", "their", "i", "me", "we", "you", "he", "she", "they",
        "it", "to", "of", "in", "for", "on", "with", "at", "by", "from",
        "and", "or", "but", "not", "no", "about", "what", "where", "when",
        "who", "how", "which", "that", "this", "these", "those",
    ].into_iter().collect()
});

use crate::graph_backend::GraphBackend;
use crate::models::{EdgeRow, EntityRow, SupersessionRecord, EMBEDDING_DIM};

const DEFAULT_POOL_SIZE: usize = 4;

#[derive(Clone)]
pub struct GraphClient {
    pool: Arc<Vec<Mutex<AsyncGraph>>>,
    next: Arc<std::sync::atomic::AtomicUsize>,
    graph_name: String,
}

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

impl GraphRegistry {
    pub async fn connect(connection_string: &str, default_graph: &str) -> Result<Self> {
        let info: FalkorConnectionInfo = connection_string
            .try_into()
            .context("invalid FalkorDB connection string")?;
        let client = FalkorClientBuilder::new_async()
            .with_connection_info(info)
            .build()
            .await
            .context("failed to connect to FalkorDB")?;

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

    /// Create a registry backed by in-memory graphs (no database needed).
    pub fn in_memory(default_graph: &str) -> Self {
        let registry = Self {
            factory: Box::new(|name: &str| {
                Arc::new(crate::in_memory_graph::InMemoryGraph::new(name)) as Arc<dyn GraphBackend>
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };
        registry
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

impl GraphClient {
    pub async fn connect(connection_string: &str, graph_name: &str) -> Result<Self> {
        let info: FalkorConnectionInfo = connection_string
            .try_into()
            .context("invalid FalkorDB connection string")?;
        let client = FalkorClientBuilder::new_async()
            .with_connection_info(info)
            .build()
            .await
            .context("failed to connect to FalkorDB")?;
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
        graph.query("RETURN 1").execute().await.context("ping failed")?;
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
        let tokens: Vec<&str> = query_str.split_whitespace()
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
            let clean: String = token.chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '\'')
                .collect();
            if clean.is_empty() { continue; }
            let safe = clean.replace('\'', "\\'");
            let query = format!(
                "CALL db.idx.fulltext.queryNodes('Entity', '{}') YIELD node RETURN node LIMIT 5",
                safe
            );
            match graph.query(&query).execute().await {
                Ok(result) => {
                    let rows: Vec<Vec<FalkorValue>> = result.data.collect();
                    for row in rows {
                        if let Some(entity) = row.into_iter().next().and_then(|v| node_to_entity_row(v)) {
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
        let result = graph.query(&query).execute().await
            .context("vector search entities failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 2 { return None; }
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
        let result = graph.query(&query).execute().await
            .context("vector search edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 4 { return None; }
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
            let result = graph.query(&query).execute().await
                .context("edge contains search failed")?;
            let rows: Vec<Vec<FalkorValue>> = result.data.collect();
            for row in rows {
                let Ok([rel, src, dst]) = take_n(row) else { continue };
                match edge_row_from_values(rel, src, dst) {
                    Ok(e) if seen.insert(e.edge_id) => results.push(e),
                    _ => {}
                }
            }
        }
        Ok(results)
    }

    pub async fn fulltext_search_edges_at(&self, query_str: &str, at: DateTime<Utc>) -> Result<Vec<EdgeRow>> {
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
            let result = graph.query(&query).execute().await
                .context("edge contains search (temporal) failed")?;
            let rows: Vec<Vec<FalkorValue>> = result.data.collect();
            for row in rows {
                let Ok([rel, src, dst]) = take_n(row) else { continue };
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
        let result = graph.query(&query).execute().await
            .context("vector search edges (temporal) failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 4 { return None; }
                let mut it = row.into_iter();
                let rel = it.next()?;
                let src = it.next()?;
                let dst = it.next()?;
                let _score = it.next()?;
                Some(edge_row_from_values(rel, src, dst))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn walk_one_hop_at(&self, entity_ids: &[String], limit: usize, at: DateTime<Utc>) -> Result<Vec<EdgeRow>> {
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

    pub async fn entity_timeline(&self, entity_name: &str) -> Result<Vec<EdgeRow>> {
        let safe = sanitise(entity_name);
        let query = format!(
            "MATCH (a)-[r:RELATION]->(b) \
             WHERE toLower(a.name) = toLower('{safe}') OR toLower(b.name) = toLower('{safe}') \
             RETURN r, a, b \
             ORDER BY r.valid_at ASC"
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
        let result = graph.query(query).execute().await
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
        graph.query(&query).execute().await.context("upsert entity failed")?;
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
            rel.source_agents.iter().map(|s| sanitise(s)).collect::<Vec<_>>().join("|"),
            rel.valid_at.to_rfc3339(),
            rel.confidence,
            rel.created_at.to_rfc3339(),
            rel.created_at.to_rfc3339(),
            rel.confidence,
            rel.memory_tier,
        );
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await.context("create edge failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            if let Some(v) = row.into_iter().next() {
                return Ok(extract_int(&v));
            }
        }
        Ok(-1)
    }

    pub async fn promote_working_memory(&self) -> Result<usize> {
        // FalkorDB: use ISO string comparison instead of datetime() + duration()
        let one_hour_ago = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let one_hour_ago = &one_hour_ago[..19]; // strip tz for string compare
        let query = format!("\
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
            RETURN count(r) AS promoted");
        let mut graph = self.conn().lock().await;
        let result = graph.query(query).execute().await
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
        let now_iso = &now.to_rfc3339()[..19];
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
        let result = graph.query(&query).execute().await
            .context("expire TTL edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            if let Some(v) = row.into_iter().next() {
                return Ok(extract_int(&v) as usize);
            }
        }
        Ok(0)
    }

    pub async fn memory_tier_stats(&self) -> Result<(usize, usize)> {
        let query = "\
            MATCH ()-[r:RELATION]-() \
            WHERE r.invalid_at IS NULL \
              AND (r.archived IS NULL OR r.archived = false) \
            RETURN \
              sum(CASE WHEN r.memory_tier = 'working' THEN 1 ELSE 0 END) AS working, \
              sum(CASE WHEN r.memory_tier = 'long_term' OR r.memory_tier IS NULL THEN 1 ELSE 0 END) AS long_term";
        let mut graph = self.conn().lock().await;
        let result = graph.query(query).execute().await
            .context("memory tier stats failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        if let Some(row) = rows.into_iter().next() {
            let mut iter = row.into_iter();
            let working = iter.next().map(|v| extract_int(&v) as usize).unwrap_or(0);
            let long_term = iter.next().map(|v| extract_int(&v) as usize).unwrap_or(0);
            return Ok((working, long_term));
        }
        Ok((0, 0))
    }

    pub async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        let query = format!(
            "MATCH ()-[r:RELATION]->() WHERE id(r) = {edge_id} \
             SET r.invalid_at = '{}'",
            at.to_rfc3339()
        );
        let mut graph = self.conn().lock().await;
        graph.query(&query).execute().await.context("invalidate edge failed")?;
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
        let result = graph.query(&query).execute().await
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

    pub async fn increment_salience(&self, edge_ids: &[i64]) -> Result<()> {
        if edge_ids.is_empty() {
            return Ok(());
        }
        let ids_list = edge_ids
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let now_iso = Utc::now().to_rfc3339();
        let query = format!(
            "MATCH ()-[r:RELATION]->() WHERE id(r) IN [{ids_list}] \
             SET r.salience = r.salience + 1, r.last_accessed_at = '{now_iso}'"
        );
        let mut graph = self.conn().lock().await;
        graph.query(&query).execute().await.context("increment salience failed")?;
        Ok(())
    }

    pub async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<usize> {
        let stale_before_iso = stale_before.to_rfc3339();
        let now_iso = now.to_rfc3339();
        // FalkorDB localdatetime() needs no timezone offset — strip to "YYYY-MM-DDTHH:MM:SS"
        let now_no_tz = &now_iso[..19];
        // FalkorDB doesn't support duration.between(); compute approx days from date components
        let query = format!(
            "MATCH ()-[r:RELATION]-() \
             WHERE r.invalid_at IS NULL \
               AND r.last_accessed_at < '{stale_before_iso}' \
             WITH r, \
               localdatetime(substring(r.last_accessed_at, 0, 19)) AS accessed_dt, \
               localdatetime('{now_no_tz}') AS now_dt \
             WITH r, \
               toFloat((now_dt.year - accessed_dt.year) * 365 \
                     + (now_dt.month - accessed_dt.month) * 30 \
                     + (now_dt.day - accessed_dt.day)) AS days_stale \
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
        let result = graph.query(&query).execute().await
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
        let result = graph.query(&query).execute().await
            .context("find close unlinked failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| {
                if row.len() < 2 { return None; }
                let score = extract_float(&row[1]);
                node_to_entity_row(row.into_iter().next()?).map(|e| e.map(|e| (e, score)))
            })
            .collect::<Result<Vec<_>>>()
    }

    pub async fn find_placeholder_nodes(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<EntityRow>> {
        let query = format!(
            "MATCH (e:Entity) \
             WHERE e.resolved = false AND e.created_at < '{}' \
             RETURN e \
             LIMIT 20",
            cutoff.to_rfc3339()
        );
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await
            .context("find placeholders failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| row.into_iter().next().and_then(|v| node_to_entity_row(v)))
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
        graph.query(&q1).execute().await.context("merge placeholder step 1 failed")?;

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
        graph.query(&q2).execute().await.context("merge placeholder step 2 failed")?;

        let q3 = format!(
            "MATCH (p:Entity {{id: '{placeholder_id}'}}) DELETE p"
        );
        graph.query(&q3).execute().await.context("merge placeholder step 3 failed")?;

        Ok(())
    }

    /// Return all ACTIVE facts for a specific entity (subject or object), ordered by confidence desc.
    pub async fn entity_facts(&self, entity_name: &str) -> Result<Vec<EdgeRow>> {
        // Fulltext search for the entity, then walk one hop on found IDs
        let ft_entities = self.fulltext_search_entities(entity_name).await?;
        if ft_entities.is_empty() {
            return Ok(vec![]);
        }
        let query_lower = entity_name.to_lowercase();
        let ids: Vec<String> = ft_entities.into_iter()
            .filter(|e| {
                let name_lower = e.name.to_lowercase();
                name_lower == query_lower
                    || name_lower.starts_with(&query_lower)
                    || name_lower.contains(&query_lower)
            })
            .map(|e| e.id)
            .collect();
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut edges = self.walk_one_hop(&ids, 100).await?;
        edges.retain(|e| e.invalid_at.is_none());
        edges.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        Ok(edges)
    }

    /// Return graph statistics: (entity_count, fact_count, oldest_valid_at, newest_valid_at, avg_confidence).
    pub async fn graph_stats(&self) -> Result<(usize, usize, Option<String>, Option<String>, f32)> {
        let mut graph = self.conn().lock().await;

        // Entity count
        let result = graph.query("MATCH (e:Entity) RETURN count(e) AS entity_count")
            .execute().await.context("graph_stats entity count failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let entity_count = rows.into_iter().next()
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
            let fact_count = vals.get(0).map(extract_int).unwrap_or(0) as usize;
            let oldest = vals.get(1).and_then(|v| {
                let s = extract_string(v);
                if s.is_empty() { None } else { Some(s) }
            });
            let newest = vals.get(2).and_then(|v| {
                let s = extract_string(v);
                if s.is_empty() { None } else { Some(s) }
            });
            let avg_conf = vals.get(3).map(extract_float).unwrap_or(0.0);
            Ok((entity_count, fact_count, oldest, newest, avg_conf))
        } else {
            Ok((entity_count, 0, None, None, 0.0))
        }
    }

    /// Return all distinct relation_types present in the graph (active edges only).
    pub async fn all_relation_types(&self) -> Result<Vec<String>> {
        let mut graph = self.conn().lock().await;
        let result = graph.query(
            "MATCH ()-[r:RELATION]-() WHERE r.invalid_at IS NULL \
             RETURN DISTINCT r.relation_type ORDER BY r.relation_type"
        ).execute().await.context("all_relation_types failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        Ok(rows.into_iter()
            .filter_map(|r| r.into_iter().next().map(|v| extract_string(&v)))
            .filter(|s| !s.is_empty())
            .collect())
    }

    /// Return entities with fewer than `threshold` active edges.
    pub async fn under_documented_entities(&self, threshold: usize) -> Result<Vec<(String, String, usize)>> {
        let mut graph = self.conn().lock().await;
        let query = format!(
            "MATCH (e:Entity) \
             OPTIONAL MATCH (e)-[r:RELATION]-() WHERE r.invalid_at IS NULL \
             WITH e, count(r) AS edge_count \
             WHERE edge_count < {threshold} \
             RETURN e.name, e.entity_type, edge_count \
             ORDER BY edge_count ASC \
             LIMIT 20"
        );
        let result = graph.query(&query).execute().await
            .context("under_documented_entities failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        Ok(rows.into_iter()
            .filter_map(|row| {
                let vals: Vec<FalkorValue> = row;
                let name = vals.get(0).map(extract_string)?;
                let entity_type = vals.get(1).map(extract_string).unwrap_or_default();
                let count = vals.get(2).map(extract_int).unwrap_or(0) as usize;
                Some((name, entity_type, count))
            })
            .collect())
    }

    /// Return entity type counts for stats.
    pub async fn entity_type_counts(&self) -> Result<std::collections::HashMap<String, usize>> {
        let mut graph = self.conn().lock().await;
        let result = graph.query(
            "MATCH (e:Entity) RETURN e.entity_type, count(e) ORDER BY count(e) DESC"
        ).execute().await.context("entity_type_counts failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let vals: Vec<FalkorValue> = row;
            let etype = vals.get(0).map(extract_string).unwrap_or_default();
            let count = vals.get(1).map(extract_int).unwrap_or(0) as usize;
            if !etype.is_empty() {
                map.insert(etype, count);
            }
        }
        Ok(map)
    }

    pub async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        let query = "MATCH (e:Entity) RETURN e ORDER BY e.name".to_string();
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await.context("dump entities failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| row.into_iter().next().and_then(|v| node_to_entity_row(v)))
            .collect()
    }

    pub async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        let query = "MATCH (a)-[r:RELATION]->(b) RETURN r, a, b ORDER BY a.name".to_string();
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await.context("dump edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let mut out = Vec::new();
        for row in rows {
            let Ok([rel, src, dst]) = take_n(row) else { continue };
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
        let result = graph.query(&query).execute().await.context("get entity facts failed")?;
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
        graph.query(&query).execute().await.context("create supersession failed")?;
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
        let result = graph.query(&query).execute().await
            .context("get supersession chain failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .map(|row| supersession_from_row(row))
            .collect()
    }

    pub async fn get_provenance(&self, edge_id: i64) -> Result<crate::models::ProvenanceResponse> {
        let chain = self.get_supersession_chain(edge_id).await?;
        let superseded_by = chain.iter()
            .find(|s| s.old_edge_id == edge_id)
            .cloned();
        let supersedes: Vec<SupersessionRecord> = chain.into_iter()
            .filter(|s| s.new_edge_id == edge_id)
            .collect();
        Ok(crate::models::ProvenanceResponse {
            edge_id,
            superseded_by,
            supersedes,
        })
    }

    /// Find entity pairs that are 2-hop connected but have no direct edge.
    pub async fn find_two_hop_unlinked_pairs(&self, limit: usize) -> Result<Vec<(EntityRow, EntityRow)>> {
        let query = format!(
            "MATCH (a:Entity)-[:RELATION*2]-(b:Entity) \
             WHERE NOT (a)-[:RELATION]-(b) AND a.id <> b.id \
             RETURN DISTINCT a, b LIMIT {limit}"
        );
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await
            .context("find two hop unlinked pairs failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let mut pairs = Vec::new();
        for row in rows {
            let Ok([a_val, b_val]) = take_n(row) else { continue };
            if let (Some(Ok(a)), Some(Ok(b))) = (node_to_entity_row(a_val), node_to_entity_row(b_val)) {
                pairs.push((a, b));
            }
        }
        Ok(pairs)
    }

    /// Archive (not delete!) edges below confidence threshold.
    /// Returns the edges that were (or would be) archived.
    pub async fn archive_low_confidence_edges(&self, threshold: f32, dry_run: bool) -> Result<Vec<EdgeRow>> {
        // First, find edges below threshold
        let query = format!(
            "MATCH (a)-[r:RELATION]->(b) \
             WHERE r.invalid_at IS NULL \
               AND (r.archived IS NULL OR r.archived = false) \
               AND r.decayed_confidence < {threshold} \
             RETURN r, a, b"
        );
        let edges = self.query_edge_rows(&query).await?;

        if !dry_run && !edges.is_empty() {
            let ids_list = edges.iter()
                .map(|e| e.edge_id.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let archive_query = format!(
                "MATCH ()-[r:RELATION]->() WHERE id(r) IN [{ids_list}] \
                 SET r.archived = true"
            );
            let mut graph = self.conn().lock().await;
            graph.query(&archive_query).execute().await
                .context("archive low confidence edges failed")?;
        }

        Ok(edges)
    }

    /// Find connected components / clusters using edges, returning clusters with >= min_size entities.
    pub async fn find_entity_clusters(&self, min_size: usize) -> Result<Vec<Vec<String>>> {
        // Get all active, non-archived edges
        let query = "MATCH (a)-[r:RELATION]->(b) \
                     WHERE r.invalid_at IS NULL \
                       AND (r.archived IS NULL OR r.archived = false) \
                     RETURN DISTINCT a.name, b.name";
        let mut graph = self.conn().lock().await;
        let result = graph.query(query).execute().await
            .context("find entity clusters failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        drop(graph);

        // Build adjacency list
        let mut adj: std::collections::HashMap<String, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for row in rows {
            if row.len() < 2 { continue; }
            let a = extract_string(&row[0]);
            let b = extract_string(&row[1]);
            if a.is_empty() || b.is_empty() { continue; }
            adj.entry(a.clone()).or_default().insert(b.clone());
            adj.entry(b).or_default().insert(a);
        }

        // BFS to find connected components
        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut clusters = Vec::new();

        for start in adj.keys() {
            if visited.contains(start) { continue; }
            let mut component = Vec::new();
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(start.clone());
            visited.insert(start.clone());

            while let Some(node) = queue.pop_front() {
                component.push(node.clone());
                if let Some(neighbors) = adj.get(&node) {
                    for neighbor in neighbors {
                        if visited.insert(neighbor.clone()) {
                            queue.push_back(neighbor.clone());
                        }
                    }
                }
            }

            if component.len() >= min_size {
                component.sort();
                clusters.push(component);
            }
        }

        clusters.sort_by(|a, b| b.len().cmp(&a.len()));
        Ok(clusters)
    }

    pub async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        let entity_id = sanitise(entity_id);
        let query = format!(
            "MATCH (e:Entity {{id: '{entity_id}'}}) RETURN e"
        );
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await.context("get entity by id failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        Ok(rows.into_iter()
            .filter_map(|row| row.into_iter().next().and_then(|v| node_to_entity_row(v)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .next())
    }

    pub async fn save_source_credibility(&self, cred: &crate::credibility::SourceCredibility) -> Result<()> {
        let agent_id = sanitise(&cred.agent_id);
        let updated_at = chrono::Utc::now().to_rfc3339();
        let query = format!(
            "MERGE (s:SourceCredibility {{agent_id: '{agent_id}'}}) \
             SET s.credibility = {}, \
                 s.fact_count = {}, \
                 s.contradiction_rate = {}, \
                 s.updated_at = '{updated_at}'",
            cred.credibility,
            cred.fact_count,
            cred.contradiction_rate,
        );
        let mut graph = self.conn().lock().await;
        graph.query(&query).execute().await.context("save source credibility failed")?;
        Ok(())
    }

    pub async fn load_all_source_credibility(&self) -> Result<Vec<crate::credibility::SourceCredibility>> {
        let query = "MATCH (s:SourceCredibility) \
                     RETURN s.agent_id, s.credibility, s.fact_count, s.contradiction_rate";
        let mut graph = self.conn().lock().await;
        let result = graph.query(query).execute().await
            .context("load source credibility failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let mut out = Vec::new();
        for row in rows {
            let Ok([a, b, c, d]) = take_n(row) else { continue };
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
        let result = graph.query(&invalidate_query).execute().await
            .context("delete_entity: invalidate edges failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        let count = rows.into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .map(|v| extract_int(&v) as usize)
            .unwrap_or(0);

        // Delete the entity node
        let delete_query = format!(
            "MATCH (e:Entity {{id: '{entity_id}'}}) DELETE e"
        );
        graph.query(&delete_query).execute().await
            .context("delete_entity: delete node failed")?;

        Ok(count)
    }

    pub async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        let entity_id = sanitise(entity_id);
        let new_name = sanitise(new_name);
        let query = format!(
            "MATCH (e:Entity {{id: '{entity_id}'}}) SET e.name = '{new_name}'"
        );
        let mut graph = self.conn().lock().await;
        graph.query(&query).execute().await
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
        let query = format!(
            "MATCH (e:Entity {{id: '{entity_id}'}}) SET e.`{key}` = {cypher_value}"
        );
        let mut graph = self.conn().lock().await;
        graph.query(&query).execute().await
            .context("set_entity_property failed")?;
        Ok(())
    }

    pub async fn list_entities_by_recency(&self, offset: usize, limit: usize) -> Result<Vec<EntityRow>> {
        let query = format!(
            "MATCH (e:Entity) RETURN e ORDER BY e.created_at DESC SKIP {offset} LIMIT {limit}"
        );
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await
            .context("list_entities_by_recency failed")?;
        let rows: Vec<Vec<FalkorValue>> = result.data.collect();
        rows.into_iter()
            .filter_map(|row| row.into_iter().next())
            .filter_map(|val| node_to_entity_row(val))
            .collect::<Result<Vec<_>>>()
    }

    pub async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> {
        let key = sanitise(key);
        let value = sanitise(value);
        // Match both string and boolean forms (e.g. is_principal = 'true' OR is_principal = true)
        let query = format!(
            "MATCH (e:Entity) WHERE e.`{key}` = '{value}' OR e.`{key}` = {value} RETURN e LIMIT 1"
        );
        let mut graph = self.conn().lock().await;
        let result = graph.query(&query).execute().await
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

#[async_trait]
impl GraphBackend for GraphClient {
    fn graph_name(&self) -> &str { &self.graph_name }
    async fn ping(&self) -> Result<()> { self.ping().await }
    async fn setup_schema(&self) -> Result<()> { self.setup_schema().await }
    async fn drop_and_reinitialise(&self) -> Result<()> { self.drop_and_reinitialise().await }
    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> { self.fulltext_search_entities(query_str).await }
    async fn vector_search_entities(&self, embedding: &[f32], k: usize) -> Result<Vec<(EntityRow, f32)>> { self.vector_search_entities(embedding, k).await }
    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> { self.get_entity_by_id(entity_id).await }
    async fn fulltext_search_edges(&self, query_str: &str) -> Result<Vec<EdgeRow>> { self.fulltext_search_edges(query_str).await }
    async fn fulltext_search_edges_at(&self, query_str: &str, at: DateTime<Utc>) -> Result<Vec<EdgeRow>> { self.fulltext_search_edges_at(query_str, at).await }
    async fn vector_search_edges_scored(&self, embedding: &[f32], k: usize) -> Result<Vec<(EdgeRow, f32)>> { self.vector_search_edges_scored(embedding, k).await }
    async fn vector_search_edges_at(&self, embedding: &[f32], k: usize, at: DateTime<Utc>) -> Result<Vec<EdgeRow>> { self.vector_search_edges_at(embedding, k, at).await }
    async fn walk_one_hop(&self, entity_ids: &[String], limit: usize) -> Result<Vec<EdgeRow>> { self.walk_one_hop(entity_ids, limit).await }
    async fn walk_one_hop_at(&self, entity_ids: &[String], limit: usize, at: DateTime<Utc>) -> Result<Vec<EdgeRow>> { self.walk_one_hop_at(entity_ids, limit, at).await }
    async fn walk_n_hops(&self, seed_entity_ids: &[String], max_hops: usize, limit_per_hop: usize) -> Result<Vec<(EdgeRow, usize)>> { self.walk_n_hops(seed_entity_ids, max_hops, limit_per_hop).await }
    async fn entity_timeline(&self, entity_name: &str) -> Result<Vec<EdgeRow>> { self.entity_timeline(entity_name).await }
    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> { self.find_all_active_edges_from(node_id).await }
    async fn upsert_entity(&self, entity: &crate::models::Entity) -> Result<()> { self.upsert_entity(entity).await }
    async fn create_edge(&self, from_id: &str, to_id: &str, rel: &crate::models::Relation) -> Result<i64> { self.create_edge(from_id, to_id, rel).await }
    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> { self.invalidate_edge(edge_id, at).await }
    async fn compound_edge_confidence(&self, edge_id: i64, new_agent: &str, new_confidence: f32) -> Result<f32> { self.compound_edge_confidence(edge_id, new_agent, new_confidence).await }
    async fn increment_salience(&self, edge_ids: &[i64]) -> Result<()> { self.increment_salience(edge_ids).await }
    async fn delete_entity(&self, entity_id: &str) -> Result<usize> { self.delete_entity(entity_id).await }
    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> { self.merge_placeholder(placeholder_id, resolved_id).await }
    async fn promote_working_memory(&self) -> Result<usize> { self.promote_working_memory().await }
    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> { self.expire_ttl_edges(now).await }
    async fn memory_tier_stats(&self) -> Result<(usize, usize)> { self.memory_tier_stats().await }
    async fn decay_stale_edges(&self, stale_before: DateTime<Utc>, now: DateTime<Utc>) -> Result<usize> { self.decay_stale_edges(stale_before, now).await }
    async fn entity_facts(&self, entity_name: &str) -> Result<Vec<EdgeRow>> { self.entity_facts(entity_name).await }
    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> { self.get_entity_facts(entity_id).await }
    async fn graph_stats(&self) -> Result<(usize, usize, Option<String>, Option<String>, f32)> { self.graph_stats().await }
    async fn all_relation_types(&self) -> Result<Vec<String>> { self.all_relation_types().await }
    async fn under_documented_entities(&self, threshold: usize) -> Result<Vec<(String, String, usize)>> { self.under_documented_entities(threshold).await }
    async fn entity_type_counts(&self) -> Result<HashMap<String, usize>> { self.entity_type_counts().await }
    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> { self.dump_all_entities().await }
    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> { self.dump_all_edges().await }
    async fn list_entities_by_recency(&self, offset: usize, limit: usize) -> Result<Vec<EntityRow>> { self.list_entities_by_recency(offset, limit).await }
    async fn create_supersession(&self, old_edge_id: i64, new_edge_id: i64, superseded_at: DateTime<Utc>, old_fact: &str, new_fact: &str) -> Result<()> { self.create_supersession(old_edge_id, new_edge_id, superseded_at, old_fact, new_fact).await }
    async fn get_supersession_chain(&self, edge_id: i64) -> Result<Vec<SupersessionRecord>> { self.get_supersession_chain(edge_id).await }
    async fn get_provenance(&self, edge_id: i64) -> Result<crate::models::ProvenanceResponse> { self.get_provenance(edge_id).await }
    async fn find_close_unlinked(&self, node_id: &str, embedding: &[f32], threshold: f32) -> Result<Vec<(EntityRow, f32)>> { self.find_close_unlinked(node_id, embedding, threshold).await }
    async fn find_placeholder_nodes(&self, cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> { self.find_placeholder_nodes(cutoff).await }
    async fn find_two_hop_unlinked_pairs(&self, limit: usize) -> Result<Vec<(EntityRow, EntityRow)>> { self.find_two_hop_unlinked_pairs(limit).await }
    async fn archive_low_confidence_edges(&self, threshold: f32, dry_run: bool) -> Result<Vec<EdgeRow>> { self.archive_low_confidence_edges(threshold, dry_run).await }
    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> { self.rename_entity(entity_id, new_name).await }
    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> { self.set_entity_property(entity_id, key, value).await }
    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> { self.find_entity_by_property(key, value).await }
    async fn find_entity_clusters(&self, min_size: usize) -> Result<Vec<Vec<String>>> { self.find_entity_clusters(min_size).await }
    async fn save_source_credibility(&self, cred: &crate::credibility::SourceCredibility) -> Result<()> { self.save_source_credibility(cred).await }
    async fn load_all_source_credibility(&self) -> Result<Vec<crate::credibility::SourceCredibility>> { self.load_all_source_credibility().await }
}

fn sanitise(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

// --- Value parsing helpers ---

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

fn edge_row_from_values(rel: FalkorValue, src: FalkorValue, dst: FalkorValue) -> Result<EdgeRow> {
    let (edge_id, fact, relation_type, confidence, salience, valid_at, invalid_at, embedding, decayed_confidence, source_agents, memory_tier, expires_at) =
        match &rel {
            FalkorValue::Edge(edge) => {
                let p = &edge.properties;
                let confidence = prop_float(p, "confidence");
                let valid_at = prop_string(p, "valid_at");
                let decayed_confidence = {
                    let v = prop_float(p, "decayed_confidence");
                    if v == 0.0 { confidence } else { v }
                };
                let source_agents = prop_opt_string(p, "source_agents").unwrap_or_default();
                let memory_tier = prop_opt_string(p, "memory_tier").unwrap_or_else(|| "long_term".to_string());
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

fn prop_string(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> String {
    p.get(key).map(extract_string).unwrap_or_default()
}

fn prop_opt_string(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> Option<String> {
    p.get(key).and_then(|v| match v {
        FalkorValue::None => None,
        FalkorValue::String(s) if s.is_empty() => None,
        other => Some(extract_string(other)),
    })
}

fn prop_bool(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> bool {
    p.get(key).map(extract_bool).unwrap_or(false)
}

fn prop_float(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> f32 {
    p.get(key).map(extract_float).unwrap_or(0.0)
}

fn prop_int(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> i64 {
    p.get(key).map(extract_int).unwrap_or(0)
}

fn prop_embedding(p: &std::collections::HashMap<String, FalkorValue>, key: &str) -> Vec<f32> {
    p.get(key).map(extract_embedding).unwrap_or_else(|| vec![0.0; EMBEDDING_DIM])
}

fn extract_string(v: &FalkorValue) -> String {
    match v {
        FalkorValue::String(s) => s.clone(),
        FalkorValue::I64(i) => i.to_string(),
        FalkorValue::F64(f) => f.to_string(),
        FalkorValue::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

fn extract_bool(v: &FalkorValue) -> bool {
    match v {
        FalkorValue::Bool(b) => *b,
        FalkorValue::String(s) => s == "true",
        FalkorValue::I64(i) => *i != 0,
        _ => false,
    }
}

fn extract_int(v: &FalkorValue) -> i64 {
    match v {
        FalkorValue::I64(i) => *i,
        FalkorValue::F64(f) => *f as i64,
        _ => 0,
    }
}

fn extract_float(v: &FalkorValue) -> f32 {
    match v {
        FalkorValue::F64(f) => *f as f32,
        FalkorValue::I64(i) => *i as f32,
        _ => 0.0,
    }
}

fn extract_embedding(v: &FalkorValue) -> Vec<f32> {
    match v {
        FalkorValue::Array(arr) => arr.iter().map(extract_float).collect(),
        FalkorValue::Vec32(v32) => v32.values.clone(),
        _ => vec![0.0; EMBEDDING_DIM],
    }
}

/// Destructure a row into a fixed-size array, bailing if too short.
fn take_n<const N: usize>(row: Vec<FalkorValue>) -> Result<[FalkorValue; N]> {
    row.try_into()
        .map_err(|v: Vec<FalkorValue>| anyhow::anyhow!("expected {N} columns, got {}", v.len()))
}

fn supersession_from_row(row: Vec<FalkorValue>) -> Result<SupersessionRecord> {
    let [a, b, c, d, e] = take_n(row)?;
    let old_edge_id = extract_int(&a);
    let new_edge_id = extract_int(&b);
    let superseded_at_str = extract_string(&c);
    let old_fact = extract_string(&d);
    let new_fact = extract_string(&e);
    let superseded_at = superseded_at_str.parse::<DateTime<Utc>>()
        .unwrap_or_else(|_| Utc::now());
    Ok(SupersessionRecord {
        old_edge_id,
        new_edge_id,
        superseded_at,
        old_fact,
        new_fact,
    })
}

fn vec_literal(v: &[f32]) -> String {
    let inner = v.iter()
        .map(|f| format!("{:.6}", f))
        .collect::<Vec<_>>()
        .join(", ");
    format!("vecf32([{inner}])")
}
