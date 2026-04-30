use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use rusqlite::{params, Connection};

// NOTE on async correctness:
//
// `rusqlite::Connection` is synchronous. Every method routes its blocking
// section through [`SqliteGraph::with_conn_blocking`], which uses
// `tokio::task::block_in_place` to tell the multi-threaded runtime that this
// thread is about to block, so other tasks can be moved to a different worker.
// We use `parking_lot::Mutex` (not `tokio::sync::Mutex`) because:
//   1. The lock is never held across an `.await` point.
//   2. parking_lot mutexes don't poison: a panic in one query handler does
//      not take down subsequent SQLite traffic.
//
// `block_in_place` requires the multi-threaded tokio runtime. The default
// `#[tokio::main]` (with the `full` feature, which we enable) provides one;
// `current_thread` runtimes will panic.

use crate::credibility::SourceCredibility;
use crate::graph_backend::GraphBackend;
use crate::models::{
    EdgeRow, Entity, EntityRow, MemoryTier, ProvenanceResponse, Relation, SupersessionRecord,
};

use crate::math::{compound_confidence, cosine_similarity};
use crate::models::{deserialize_embedding, serialize_embedding};

pub struct SqliteGraph {
    name: String,
    conn: Mutex<Connection>,
    next_edge_id: AtomicI64,
}

impl SqliteGraph {
    pub fn open(name: &str, path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open SQLite database at {}", path.display()))?;
        let graph = Self {
            name: name.to_string(),
            conn: Mutex::new(conn),
            next_edge_id: AtomicI64::new(1),
        };
        Ok(graph)
    }

    pub fn in_memory(name: &str) -> Result<Self> {
        let conn =
            Connection::open_in_memory().context("failed to open in-memory SQLite database")?;
        let graph = Self {
            name: name.to_string(),
            conn: Mutex::new(conn),
            next_edge_id: AtomicI64::new(1),
        };
        Ok(graph)
    }

    /// Run a synchronous closure with the SQLite connection.
    ///
    /// Wraps the blocking section in `tokio::task::block_in_place` so the
    /// runtime can move other tasks off this worker while we wait on disk I/O.
    /// All call sites in this file route through here.
    fn with_conn_blocking<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Connection) -> R,
    {
        tokio::task::block_in_place(|| {
            let conn = self.conn.lock();
            f(&conn)
        })
    }

    async fn init_next_edge_id(&self) -> Result<()> {
        self.with_conn_blocking(|conn| {
            let max_id: i64 = conn
                .query_row("SELECT COALESCE(MAX(edge_id), 0) FROM edges", [], |row| {
                    row.get(0)
                })
                .unwrap_or(0);
            self.next_edge_id.store(max_id + 1, Ordering::Relaxed);
            Ok(())
        })
    }

    async fn walk_one_hop_inner(
        &self,
        entity_ids: &[String],
        limit: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
        if entity_ids.is_empty() {
            return Ok(vec![]);
        }
        self.with_conn_blocking(|conn| {
            let placeholders: Vec<String> = entity_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let ph = placeholders.join(",");
            if let Some(at) = at {
                let at_str = at.to_rfc3339();
                let next = entity_ids.len() + 1;
                let sql = format!(
                    "{} WHERE e.valid_at <= ?{next} AND (e.invalid_at IS NULL OR e.invalid_at > ?{next})
                     AND (e.from_id IN ({ph}) OR e.to_id IN ({ph})) LIMIT ?{}",
                    EDGES_SELECT,
                    next + 1
                );
                let mut stmt = conn.prepare(&sql)?;
                let limit_i64 = limit as i64;
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = entity_ids
                    .iter()
                    .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                param_values.push(Box::new(at_str));
                param_values.push(Box::new(limit_i64));
                let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(|b| b.as_ref()).collect();
                let rows = stmt
                    .query_map(params_ref.as_slice(), row_to_edge)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            } else {
                let sql = format!(
                    "{} WHERE e.invalid_at IS NULL AND (e.from_id IN ({ph}) OR e.to_id IN ({ph})) LIMIT ?{}",
                    EDGES_SELECT,
                    entity_ids.len() + 1
                );
                let mut stmt = conn.prepare(&sql)?;
                let limit_i64 = limit as i64;
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = entity_ids
                    .iter()
                    .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                    .collect();
                param_values.push(Box::new(limit_i64));
                let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(|b| b.as_ref()).collect();
                let rows = stmt
                    .query_map(params_ref.as_slice(), row_to_edge)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            }
        })
    }
}

fn row_to_entity(row: &rusqlite::Row) -> rusqlite::Result<EntityRow> {
    let embedding_blob: Vec<u8> = row.get("embedding")?;
    Ok(EntityRow {
        id: row.get("id")?,
        name: row.get("name")?,
        entity_type: row.get("entity_type")?,
        resolved: row.get("resolved")?,
        hint: row.get("hint")?,
        content: row.get("content")?,
        created_at: row.get("created_at")?,
        embedding: deserialize_embedding(&embedding_blob),
    })
}

fn row_to_edge(row: &rusqlite::Row) -> rusqlite::Result<EdgeRow> {
    let embedding_blob: Vec<u8> = row.get("embedding")?;
    Ok(EdgeRow {
        edge_id: row.get("edge_id")?,
        subject_id: row.get("subject_id")?,
        subject_name: row.get("subject_name")?,
        fact: row.get("fact")?,
        relation_type: row.get("relation_type")?,
        confidence: row.get("confidence")?,
        salience: row.get("salience")?,
        valid_at: row.get("valid_at")?,
        invalid_at: row.get("invalid_at")?,
        object_id: row.get("object_id")?,
        object_name: row.get("object_name")?,
        embedding: deserialize_embedding(&embedding_blob),
        decayed_confidence: row.get("decayed_confidence")?,
        source_agents: row.get("source_agents")?,
        memory_tier: row.get("memory_tier")?,
        expires_at: row.get("expires_at")?,
    })
}

const EDGES_SELECT: &str = r#"
    SELECT e.edge_id, e.from_id AS subject_id, ef.name AS subject_name,
           e.fact, e.relation_type, e.confidence, e.salience,
           e.valid_at, e.invalid_at,
           e.to_id AS object_id, et.name AS object_name,
           e.embedding, e.decayed_confidence, e.source_agents,
           e.memory_tier, e.expires_at
    FROM edges e
    LEFT JOIN entities ef ON ef.id = e.from_id
    LEFT JOIN entities et ON et.id = e.to_id
"#;

#[async_trait]
impl GraphBackend for SqliteGraph {
    fn graph_name(&self) -> &str {
        &self.name
    }

    async fn ping(&self) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute_batch("SELECT 1")?;
            Ok(())
        })
    }

    async fn setup_schema(&self) -> Result<()> {
        self.with_conn_blocking(|conn| -> Result<()> {
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS entities (
                        id TEXT PRIMARY KEY,
                        name TEXT NOT NULL,
                        entity_type TEXT NOT NULL,
                        resolved INTEGER NOT NULL DEFAULT 1,
                        hint TEXT,
                        content TEXT,
                        created_at TEXT NOT NULL,
                        embedding BLOB NOT NULL DEFAULT X''
                    );

                    CREATE TABLE IF NOT EXISTS edges (
                        edge_id INTEGER PRIMARY KEY,
                        from_id TEXT NOT NULL,
                        to_id TEXT NOT NULL,
                        fact TEXT NOT NULL,
                        relation_type TEXT NOT NULL,
                        confidence REAL NOT NULL,
                        salience INTEGER NOT NULL DEFAULT 0,
                        valid_at TEXT NOT NULL,
                        invalid_at TEXT,
                        embedding BLOB NOT NULL DEFAULT X'',
                        source_agents TEXT NOT NULL DEFAULT '',
                        memory_tier TEXT NOT NULL DEFAULT 'working',
                        created_at TEXT NOT NULL,
                        decayed_confidence REAL NOT NULL,
                        expires_at TEXT,
                        FOREIGN KEY (from_id) REFERENCES entities(id),
                        FOREIGN KEY (to_id) REFERENCES entities(id)
                    );

                    CREATE TABLE IF NOT EXISTS supersessions (
                        old_edge_id INTEGER NOT NULL,
                        new_edge_id INTEGER NOT NULL,
                        superseded_at TEXT NOT NULL,
                        old_fact TEXT NOT NULL,
                        new_fact TEXT NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS properties (
                        entity_id TEXT NOT NULL,
                        key TEXT NOT NULL,
                        value TEXT NOT NULL,
                        PRIMARY KEY (entity_id, key)
                    );

                    CREATE TABLE IF NOT EXISTS source_credibility (
                        agent_id TEXT PRIMARY KEY,
                        credibility REAL NOT NULL,
                        fact_count INTEGER NOT NULL,
                        contradiction_rate REAL NOT NULL
                    );

                    CREATE TABLE IF NOT EXISTS last_visited (
                        entity_id TEXT PRIMARY KEY,
                        visited_at TEXT NOT NULL,
                        FOREIGN KEY (entity_id) REFERENCES entities(id)
                    );

                    CREATE TABLE IF NOT EXISTS retraction_reasons (
                        edge_id INTEGER PRIMARY KEY,
                        reason TEXT NOT NULL,
                        retracted_at TEXT NOT NULL
                    );

                CREATE INDEX IF NOT EXISTS idx_edges_from ON edges(from_id);
                CREATE INDEX IF NOT EXISTS idx_edges_to ON edges(to_id);
                CREATE INDEX IF NOT EXISTS idx_entities_name ON entities(name COLLATE NOCASE);
                CREATE INDEX IF NOT EXISTS idx_edges_relation ON edges(relation_type);
                CREATE INDEX IF NOT EXISTS idx_last_visited_at ON last_visited(visited_at);
                ",
            )?;
            Ok(())
        })?;
        self.init_next_edge_id().await?;
        Ok(())
    }

    async fn drop_and_reinitialise(&self) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute_batch(
                "DELETE FROM edges;
                 DELETE FROM entities;
                 DELETE FROM supersessions;
                 DELETE FROM properties;
                 DELETE FROM source_credibility;
                 DELETE FROM last_visited;
                 DELETE FROM retraction_reasons;",
            )?;
            self.next_edge_id.store(1, Ordering::Relaxed);
            Ok(())
        })
    }

    // --- Entity search ---

    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> {
        self.with_conn_blocking(|conn| {
            let pattern = format!("%{}%", query_str.to_lowercase());
            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                 FROM entities WHERE LOWER(name) LIKE ?1",
            )?;
            let rows = stmt
                .query_map(params![pattern], row_to_entity)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    async fn vector_search_entities(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EntityRow, f32)>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                 FROM entities",
            )?;
            let entities = stmt
                .query_map([], row_to_entity)?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut scored: Vec<(EntityRow, f32)> = entities
                .into_iter()
                .filter(|e| !e.embedding.is_empty())
                .map(|e| {
                    let score = cosine_similarity(embedding, &e.embedding);
                    (e, score)
                })
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.truncate(k);
            Ok(scored)
        })
    }

    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                 FROM entities WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(params![entity_id], row_to_entity)?;
            Ok(rows.next().transpose()?)
        })
    }

    // --- Edge search ---

    async fn fulltext_search_edges(
        &self,
        query_str: &str,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
        self.with_conn_blocking(|conn| {
            let pattern = format!("%{}%", query_str.to_lowercase());
            if let Some(at) = at {
                let at_str = at.to_rfc3339();
                let sql = format!(
                    "{} WHERE e.valid_at <= ?1 AND (e.invalid_at IS NULL OR e.invalid_at > ?1)
                     AND LOWER(e.fact) LIKE ?2",
                    EDGES_SELECT
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(params![at_str, pattern], row_to_edge)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            } else {
                let sql = format!(
                    "{} WHERE e.invalid_at IS NULL AND LOWER(e.fact) LIKE ?1",
                    EDGES_SELECT
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(params![pattern], row_to_edge)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            }
        })
    }

    async fn vector_search_edges_scored(
        &self,
        embedding: &[f32],
        k: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, f32)>> {
        self.with_conn_blocking(|conn| {
            let edges: Vec<EdgeRow> = if let Some(at) = at {
                let at_str = at.to_rfc3339();
                let sql = format!(
                    "{} WHERE e.valid_at <= ?1 AND (e.invalid_at IS NULL OR e.invalid_at > ?1)",
                    EDGES_SELECT
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(params![at_str], row_to_edge)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            } else {
                let sql = format!("{} WHERE e.invalid_at IS NULL", EDGES_SELECT);
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map([], row_to_edge)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let mut scored: Vec<(EdgeRow, f32)> = edges
                .into_iter()
                .filter(|e| !e.embedding.is_empty())
                .map(|e| {
                    let score = cosine_similarity(embedding, &e.embedding);
                    (e, score)
                })
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.truncate(k);
            Ok(scored)
        })
    }

    // --- Graph traversal ---

    async fn walk_n_hops(
        &self,
        seed_entity_ids: &[String],
        max_hops: usize,
        limit_per_hop: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, usize)>> {
        let mut results = Vec::new();
        let mut frontier: Vec<String> = seed_entity_ids.to_vec();
        let mut visited_edges: std::collections::HashSet<i64> = std::collections::HashSet::new();

        for hop in 1..=max_hops {
            let hop_edges = self
                .walk_one_hop_inner(&frontier, limit_per_hop, at)
                .await?;
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

    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> {
        self.with_conn_blocking(|conn| {
            let sql = format!(
                "{} WHERE e.invalid_at IS NULL AND (e.from_id = ?1 OR e.to_id = ?1)",
                EDGES_SELECT
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map(params![node_id], row_to_edge)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    // --- Mutation ---

    async fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        self.with_conn_blocking(|conn| {
            let embedding_blob = serialize_embedding(&entity.embedding);
            conn.execute(
                "INSERT INTO entities (id, name, entity_type, resolved, hint, content, created_at, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    entity_type = excluded.entity_type,
                    resolved = excluded.resolved,
                    hint = excluded.hint,
                    content = excluded.content,
                    embedding = excluded.embedding",
                params![
                    entity.id,
                    entity.name,
                    entity.entity_type,
                    entity.resolved,
                    entity.hint,
                    entity.content,
                    entity.created_at.to_rfc3339(),
                    embedding_blob,
                ],
            )?;
            Ok(())
        })
    }

    async fn create_edge(&self, from_id: &str, to_id: &str, rel: &Relation) -> Result<i64> {
        let edge_id = self.next_edge_id.fetch_add(1, Ordering::Relaxed);
        self.with_conn_blocking(|conn| {
            let embedding_blob = serialize_embedding(&rel.embedding);
            let source_agents = rel.source_agents.join(",");
            let memory_tier = match rel.memory_tier {
                MemoryTier::Working => "working",
                MemoryTier::LongTerm => "long_term",
            };
            conn.execute(
                "INSERT INTO edges (edge_id, from_id, to_id, fact, relation_type, confidence, salience,
                 valid_at, invalid_at, embedding, source_agents, memory_tier, created_at, decayed_confidence, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    edge_id,
                    from_id,
                    to_id,
                    rel.fact,
                    rel.relation_type,
                    rel.confidence,
                    rel.salience,
                    rel.valid_at.to_rfc3339(),
                    rel.invalid_at.map(|t| t.to_rfc3339()),
                    embedding_blob,
                    source_agents,
                    memory_tier,
                    rel.created_at.to_rfc3339(),
                    rel.confidence,
                    rel.expires_at.map(|t| t.to_rfc3339()),
                ],
            )?;
            Ok(edge_id)
        })
    }

    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute(
                "UPDATE edges SET invalid_at = ?1 WHERE edge_id = ?2",
                params![at.to_rfc3339(), edge_id],
            )?;
            Ok(())
        })
    }

    async fn delete_entity(&self, entity_id: &str) -> Result<usize> {
        self.with_conn_blocking(|conn| {
            let now = Utc::now().to_rfc3339();
            let count = conn.execute(
                "UPDATE edges SET invalid_at = ?1 WHERE (from_id = ?2 OR to_id = ?2) AND invalid_at IS NULL",
                params![now, entity_id],
            )?;
            conn.execute("DELETE FROM entities WHERE id = ?1", params![entity_id])?;
            Ok(count)
        })
    }

    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute(
                "UPDATE edges SET from_id = ?1 WHERE from_id = ?2",
                params![resolved_id, placeholder_id],
            )?;
            conn.execute(
                "UPDATE edges SET to_id = ?1 WHERE to_id = ?2",
                params![resolved_id, placeholder_id],
            )?;
            conn.execute(
                "DELETE FROM entities WHERE id = ?1",
                params![placeholder_id],
            )?;
            Ok(())
        })
    }

    // --- Memory tier management ---

    async fn promote_working_memory(&self) -> Result<usize> {
        self.with_conn_blocking(|conn| {
            let threshold = (Utc::now() - Duration::hours(1)).to_rfc3339();
            let count = conn.execute(
                "UPDATE edges SET memory_tier = 'long_term'
                 WHERE memory_tier = 'working' AND invalid_at IS NULL
                 AND salience >= 3 AND created_at < ?1",
                params![threshold],
            )?;
            Ok(count)
        })
    }

    async fn memory_tier_stats(&self) -> Result<crate::models::MemoryTierStats> {
        self.with_conn_blocking(|conn| {
            let working_count: usize = conn.query_row(
                "SELECT COUNT(*) FROM edges WHERE invalid_at IS NULL AND memory_tier = 'working'",
                [],
                |row| row.get(0),
            )?;
            let long_term_count: usize = conn.query_row(
                "SELECT COUNT(*) FROM edges WHERE invalid_at IS NULL AND memory_tier = 'long_term'",
                [],
                |row| row.get(0),
            )?;
            Ok(crate::models::MemoryTierStats {
                working_count,
                long_term_count,
            })
        })
    }

    async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> Result<usize> {
        self.with_conn_blocking(|conn| {
            let threshold = stale_before.to_rfc3339();
            let count = conn.execute(
                "UPDATE edges SET decayed_confidence = decayed_confidence * 0.95
                 WHERE invalid_at IS NULL AND valid_at < ?1",
                params![threshold],
            )?;
            Ok(count)
        })
    }

    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> {
        self.with_conn_blocking(|conn| {
            let now_str = now.to_rfc3339();
            let count = conn.execute(
                "UPDATE edges SET invalid_at = ?1
                 WHERE invalid_at IS NULL AND expires_at IS NOT NULL AND expires_at <= ?1",
                params![now_str],
            )?;
            Ok(count)
        })
    }

    // --- Facts / reflection ---

    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT fact FROM edges
                 WHERE invalid_at IS NULL AND (from_id = ?1 OR to_id = ?1)",
            )?;
            let rows = stmt
                .query_map(params![entity_id], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    async fn graph_stats(&self) -> Result<crate::models::GraphStats> {
        self.with_conn_blocking(|conn| {
            let entity_count: usize =
                conn.query_row("SELECT COUNT(*) FROM entities", [], |row| row.get(0))?;
            let edge_count: usize = conn.query_row(
                "SELECT COUNT(*) FROM edges WHERE invalid_at IS NULL",
                [],
                |row| row.get(0),
            )?;
            let oldest_valid_at: Option<String> = conn
                .query_row(
                    "SELECT MIN(valid_at) FROM edges WHERE invalid_at IS NULL",
                    [],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            let newest_valid_at: Option<String> = conn
                .query_row(
                    "SELECT MAX(valid_at) FROM edges WHERE invalid_at IS NULL",
                    [],
                    |row| row.get(0),
                )
                .ok()
                .flatten();
            let avg_confidence: f32 = conn.query_row(
                "SELECT COALESCE(AVG(confidence), 0.0) FROM edges WHERE invalid_at IS NULL",
                [],
                |row| row.get(0),
            )?;
            Ok(crate::models::GraphStats {
                entity_count,
                edge_count,
                oldest_valid_at,
                newest_valid_at,
                avg_confidence,
            })
        })
    }

    // --- Dump / pagination ---

    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding FROM entities",
            )?;
            let rows = stmt
                .query_map([], row_to_entity)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        self.with_conn_blocking(|conn| {
            let sql = EDGES_SELECT.to_string();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt
                .query_map([], row_to_edge)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    async fn list_entities_by_recency(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EntityRow>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                 FROM entities ORDER BY created_at DESC LIMIT ?1 OFFSET ?2",
            )?;
            let rows = stmt
                .query_map(params![limit as i64, offset as i64], row_to_entity)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    // --- Supersession / provenance ---

    async fn get_provenance(&self, edge_id: i64) -> Result<ProvenanceResponse> {
        self.with_conn_blocking(|conn| {
            let superseded_by: Option<SupersessionRecord> = conn
                .query_row(
                    "SELECT old_edge_id, new_edge_id, superseded_at, old_fact, new_fact
                     FROM supersessions WHERE old_edge_id = ?1 LIMIT 1",
                    params![edge_id],
                    |row| {
                        let at_str: String = row.get(2)?;
                        Ok(SupersessionRecord {
                            old_edge_id: row.get(0)?,
                            new_edge_id: row.get(1)?,
                            superseded_at: at_str
                                .parse::<DateTime<Utc>>()
                                .unwrap_or_else(|_| Utc::now()),
                            old_fact: row.get(3)?,
                            new_fact: row.get(4)?,
                        })
                    },
                )
                .ok();

            let mut stmt = conn.prepare(
                "SELECT old_edge_id, new_edge_id, superseded_at, old_fact, new_fact
                 FROM supersessions WHERE new_edge_id = ?1",
            )?;
            let supersedes = stmt
                .query_map(params![edge_id], |row| {
                    let at_str: String = row.get(2)?;
                    Ok(SupersessionRecord {
                        old_edge_id: row.get(0)?,
                        new_edge_id: row.get(1)?,
                        superseded_at: at_str
                            .parse::<DateTime<Utc>>()
                            .unwrap_or_else(|_| Utc::now()),
                        old_fact: row.get(3)?,
                        new_fact: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            Ok(ProvenanceResponse {
                edge_id,
                superseded_by,
                supersedes,
            })
        })
    }

    // --- Discovery ---

    async fn find_close_unlinked(
        &self,
        node_id: &str,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Vec<(EntityRow, f32)>> {
        // Get linked entity IDs
        self.with_conn_blocking(|conn| {
            let mut link_stmt = conn.prepare(
                "SELECT DISTINCT from_id FROM edges WHERE invalid_at IS NULL AND (from_id = ?1 OR to_id = ?1)
                 UNION
                 SELECT DISTINCT to_id FROM edges WHERE invalid_at IS NULL AND (from_id = ?1 OR to_id = ?1)",
            )?;
            let linked: std::collections::HashSet<String> = link_stmt
                .query_map(params![node_id], |row| row.get(0))?
                .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
            drop(link_stmt);

            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding FROM entities",
            )?;
            let entities = stmt
                .query_map([], row_to_entity)?
                .collect::<rusqlite::Result<Vec<_>>>()?;

            let mut results: Vec<(EntityRow, f32)> = entities
                .into_iter()
                .filter(|e| e.id != node_id && !linked.contains(&e.id) && !e.embedding.is_empty())
                .map(|e| {
                    let score = cosine_similarity(embedding, &e.embedding);
                    (e, score)
                })
                .filter(|(_, score)| *score >= threshold)
                .collect();
            results.sort_by(|a, b| b.1.total_cmp(&a.1));
            Ok(results)
        })
    }

    async fn find_placeholder_nodes(&self, _cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                 FROM entities WHERE resolved = 0",
            )?;
            let rows = stmt
                .query_map([], row_to_entity)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    // --- Archive ---

    // --- Entity updates ---

    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute(
                "UPDATE entities SET name = ?1 WHERE id = ?2",
                params![new_name, entity_id],
            )?;
            Ok(())
        })
    }

    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute(
                "INSERT INTO properties (entity_id, key, value) VALUES (?1, ?2, ?3)
                 ON CONFLICT(entity_id, key) DO UPDATE SET value = excluded.value",
                params![entity_id, key, value],
            )?;
            Ok(())
        })
    }

    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> {
        self.with_conn_blocking(|conn| {
            let entity_id: Option<String> = conn
                .query_row(
                    "SELECT entity_id FROM properties WHERE key = ?1 AND value = ?2 LIMIT 1",
                    params![key, value],
                    |row| row.get(0),
                )
                .ok();

            if let Some(eid) = entity_id {
                let mut stmt = conn.prepare(
                    "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                     FROM entities WHERE id = ?1",
                )?;
                let mut rows = stmt.query_map(params![eid], row_to_entity)?;
                Ok(rows.next().transpose()?)
            } else {
                Ok(None)
            }
        })
    }

    // --- Dreamer support ---

    async fn bump_salience(&self, edge_ids: &[i64]) -> Result<()> {
        if edge_ids.is_empty() {
            return Ok(());
        }
        let edge_ids = edge_ids.to_vec();
        self.with_conn_blocking(move |conn| {
            for id in &edge_ids {
                conn.execute(
                    "UPDATE edges SET salience = salience + 1 WHERE edge_id = ?1",
                    params![id],
                )?;
            }
            Ok(())
        })
    }

    async fn supersede_edge(&self, old_edge_id: i64, new_edge_id: i64) -> Result<()> {
        self.with_conn_blocking(move |conn| {
            // Idempotency: skip if this exact pair already exists.
            let existing: i64 = conn.query_row(
                "SELECT COUNT(*) FROM supersessions
                 WHERE old_edge_id = ?1 AND new_edge_id = ?2",
                params![old_edge_id, new_edge_id],
                |r| r.get(0),
            )?;
            if existing > 0 {
                return Ok(());
            }
            let old_fact: String = conn
                .query_row(
                    "SELECT fact FROM edges WHERE edge_id = ?1",
                    params![old_edge_id],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            let new_fact: String = conn
                .query_row(
                    "SELECT fact FROM edges WHERE edge_id = ?1",
                    params![new_edge_id],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            conn.execute(
                "INSERT INTO supersessions
                 (old_edge_id, new_edge_id, superseded_at, old_fact, new_fact)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    old_edge_id,
                    new_edge_id,
                    Utc::now().to_rfc3339(),
                    old_fact,
                    new_fact
                ],
            )?;
            Ok(())
        })
    }

    async fn retract_edge(&self, edge_id: i64, reason: Option<&str>) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let reason = reason.map(str::to_string);
        self.with_conn_blocking(move |conn| {
            conn.execute(
                "UPDATE edges SET invalid_at = ?1
                 WHERE edge_id = ?2 AND invalid_at IS NULL",
                params![now, edge_id],
            )?;
            if let Some(r) = reason {
                conn.execute(
                    "INSERT OR REPLACE INTO retraction_reasons
                     (edge_id, reason, retracted_at) VALUES (?1, ?2, ?3)",
                    params![edge_id, r, now],
                )?;
            }
            Ok(())
        })
    }

    async fn mark_visited(&self, entity_id: &str, at: DateTime<Utc>) -> Result<()> {
        let entity_id = entity_id.to_string();
        self.with_conn_blocking(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO last_visited (entity_id, visited_at)
                 VALUES (?1, ?2)",
                params![entity_id, at.to_rfc3339()],
            )?;
            Ok(())
        })
    }

    async fn last_visited(&self, entity_id: &str) -> Result<Option<DateTime<Utc>>> {
        let entity_id = entity_id.to_string();
        self.with_conn_blocking(move |conn| {
            let result = conn.query_row(
                "SELECT visited_at FROM last_visited WHERE entity_id = ?1",
                params![entity_id],
                |r| r.get::<_, String>(0),
            );
            match result {
                Ok(s) => Ok(DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|t| t.with_timezone(&Utc))),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

    // --- Clustering ---
}

impl SqliteGraph {
    /// Return the recorded retraction reason for an edge, if any.
    pub async fn retraction_reason(&self, edge_id: i64) -> Result<Option<String>> {
        self.with_conn_blocking(move |conn| {
            let result = conn.query_row(
                "SELECT reason FROM retraction_reasons WHERE edge_id = ?1",
                params![edge_id],
                |r| r.get::<_, String>(0),
            );
            match result {
                Ok(s) => Ok(Some(s)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })
    }

    pub async fn save_source_credibility(&self, cred: &SourceCredibility) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute(
                "INSERT INTO source_credibility (agent_id, credibility, fact_count, contradiction_rate)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id) DO UPDATE SET
                    credibility = excluded.credibility,
                    fact_count = excluded.fact_count,
                    contradiction_rate = excluded.contradiction_rate",
                params![
                    cred.agent_id,
                    cred.credibility,
                    cred.fact_count as i64,
                    cred.contradiction_rate
                ],
            )?;
            Ok(())
        })
    }

    pub async fn load_all_source_credibility(&self) -> Result<Vec<SourceCredibility>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT agent_id, credibility, fact_count, contradiction_rate FROM source_credibility",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(SourceCredibility {
                        agent_id: row.get(0)?,
                        credibility: row.get(1)?,
                        fact_count: row.get::<_, i64>(2)? as usize,
                        contradiction_rate: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub async fn compound_edge_confidence(
        &self,
        edge_id: i64,
        new_agent: &str,
        new_confidence: f32,
    ) -> Result<f32> {
        self.with_conn_blocking(|conn| {
            let result: Option<(f32, String)> = conn
                .query_row(
                    "SELECT confidence, source_agents FROM edges WHERE edge_id = ?1",
                    params![edge_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();

            if let Some((old_conf, agents_str)) = result {
                let combined = compound_confidence(old_conf, new_confidence);
                let new_agents = if agents_str.split(',').any(|a| a == new_agent) {
                    agents_str
                } else if agents_str.is_empty() {
                    new_agent.to_string()
                } else {
                    format!("{},{}", agents_str, new_agent)
                };
                conn.execute(
                    "UPDATE edges SET confidence = ?1, decayed_confidence = ?1, source_agents = ?2 WHERE edge_id = ?3",
                    params![combined, new_agents, edge_id],
                )?;
                Ok(combined)
            } else {
                Ok(new_confidence)
            }
        })
    }

    pub async fn create_supersession(
        &self,
        old_edge_id: i64,
        new_edge_id: i64,
        superseded_at: DateTime<Utc>,
        old_fact: &str,
        new_fact: &str,
    ) -> Result<()> {
        self.with_conn_blocking(|conn| {
            conn.execute(
                "INSERT INTO supersessions (old_edge_id, new_edge_id, superseded_at, old_fact, new_fact)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![old_edge_id, new_edge_id, superseded_at.to_rfc3339(), old_fact, new_fact],
            )?;
            Ok(())
        })
    }

    pub async fn get_supersession_chain(&self, edge_id: i64) -> Result<Vec<SupersessionRecord>> {
        self.with_conn_blocking(|conn| {
            let mut stmt = conn.prepare(
                "SELECT old_edge_id, new_edge_id, superseded_at, old_fact, new_fact
                 FROM supersessions WHERE old_edge_id = ?1 OR new_edge_id = ?1",
            )?;
            let rows = stmt
                .query_map(params![edge_id], |row| {
                    let at_str: String = row.get(2)?;
                    Ok(SupersessionRecord {
                        old_edge_id: row.get(0)?,
                        new_edge_id: row.get(1)?,
                        superseded_at: at_str
                            .parse::<DateTime<Utc>>()
                            .unwrap_or_else(|_| Utc::now()),
                        old_fact: row.get(3)?,
                        new_fact: row.get(4)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }
}
