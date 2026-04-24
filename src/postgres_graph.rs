use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

use crate::error::GraphConnectError;
use crate::graph_backend::GraphBackend;
use crate::models::{
    EdgeRow, Entity, EntityRow, MemoryTier, ProvenanceResponse, Relation, SupersessionRecord,
};

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

fn serialize_embedding(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn deserialize_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub struct PostgresGraph {
    pool: PgPool,
    graph_name: String,
}

impl PostgresGraph {
    pub async fn new(connection_string: &str, graph_name: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(connection_string)
            .await
            .map_err(|e| {
                GraphConnectError::new(format!(
                    "failed to connect to PostgreSQL at {connection_string}: {e}"
                ))
            })?;
        Ok(Self {
            pool,
            graph_name: graph_name.to_string(),
        })
    }
}

fn row_to_entity(row: &sqlx::postgres::PgRow) -> EntityRow {
    let embedding_blob: Vec<u8> = row.get("embedding");
    EntityRow {
        id: row.get("id"),
        name: row.get("name"),
        entity_type: row.get("entity_type"),
        resolved: row.get("resolved"),
        hint: row.get("hint"),
        content: row.get("content"),
        created_at: row.get("created_at"),
        embedding: deserialize_embedding(&embedding_blob),
    }
}

fn row_to_edge(row: &sqlx::postgres::PgRow) -> EdgeRow {
    let embedding_blob: Vec<u8> = row.get("embedding");
    EdgeRow {
        edge_id: row.get("edge_id"),
        subject_id: row.get("subject_id"),
        subject_name: row
            .get::<Option<String>, _>("subject_name")
            .unwrap_or_default(),
        fact: row.get("fact"),
        relation_type: row.get("relation_type"),
        confidence: row.get("confidence"),
        salience: row.get("salience"),
        valid_at: row.get("valid_at"),
        invalid_at: row.get("invalid_at"),
        object_id: row.get("object_id"),
        object_name: row
            .get::<Option<String>, _>("object_name")
            .unwrap_or_default(),
        embedding: deserialize_embedding(&embedding_blob),
        decayed_confidence: row.get("decayed_confidence"),
        source_agents: row.get("source_agents"),
        memory_tier: row.get("memory_tier"),
        expires_at: row.get("expires_at"),
    }
}

const EDGES_SELECT: &str = r#"
    SELECT e.edge_id, e.from_id AS subject_id, ef.name AS subject_name,
           e.fact, e.relation_type, e.confidence, e.salience,
           e.valid_at, e.invalid_at,
           e.to_id AS object_id, et.name AS object_name,
           e.embedding, e.decayed_confidence, e.source_agents,
           e.memory_tier, e.expires_at
    FROM edges e
    LEFT JOIN entities ef ON ef.id = e.from_id AND ef.graph_name = e.graph_name
    LEFT JOIN entities et ON et.id = e.to_id AND et.graph_name = e.graph_name
"#;

#[async_trait]
impl GraphBackend for PostgresGraph {
    fn graph_name(&self) -> &str {
        &self.graph_name
    }

    async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    async fn setup_schema(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS entities (
                id TEXT NOT NULL,
                graph_name TEXT NOT NULL,
                name TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                resolved BOOLEAN NOT NULL DEFAULT TRUE,
                hint TEXT,
                content TEXT,
                created_at TEXT NOT NULL,
                embedding BYTEA NOT NULL DEFAULT ''::BYTEA,
                PRIMARY KEY (graph_name, id)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS edges (
                edge_id BIGSERIAL PRIMARY KEY,
                graph_name TEXT NOT NULL,
                from_id TEXT NOT NULL,
                to_id TEXT NOT NULL,
                fact TEXT NOT NULL,
                relation_type TEXT NOT NULL,
                confidence REAL NOT NULL,
                salience BIGINT NOT NULL DEFAULT 0,
                valid_at TEXT NOT NULL,
                invalid_at TEXT,
                embedding BYTEA NOT NULL DEFAULT ''::BYTEA,
                source_agents TEXT NOT NULL DEFAULT '',
                memory_tier TEXT NOT NULL DEFAULT 'working',
                created_at TEXT NOT NULL,
                decayed_confidence REAL NOT NULL,
                expires_at TEXT
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS supersessions (
                graph_name TEXT NOT NULL,
                old_edge_id BIGINT NOT NULL,
                new_edge_id BIGINT NOT NULL,
                superseded_at TEXT NOT NULL,
                old_fact TEXT NOT NULL,
                new_fact TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS properties (
                graph_name TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY (graph_name, entity_id, key)
            )",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS source_credibility (
                graph_name TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                credibility REAL NOT NULL,
                fact_count BIGINT NOT NULL,
                contradiction_rate REAL NOT NULL,
                PRIMARY KEY (graph_name, agent_id)
            )",
        )
        .execute(&self.pool)
        .await?;

        // Indexes
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_edges_graph_from ON edges(graph_name, from_id)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_edges_graph_to ON edges(graph_name, to_id)")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_edges_graph_invalid ON edges(graph_name, invalid_at)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_entities_graph_name ON entities(graph_name, name)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_edges_relation ON edges(graph_name, relation_type)",
        )
        .execute(&self.pool)
        .await?;

        // GIN indexes for fulltext search
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_entities_name_gin ON entities USING GIN (to_tsvector('english', name))",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_edges_fact_gin ON edges USING GIN (to_tsvector('english', fact))",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn drop_and_reinitialise(&self) -> Result<()> {
        let g = &self.graph_name;
        sqlx::query("DELETE FROM edges WHERE graph_name = $1")
            .bind(g)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM entities WHERE graph_name = $1")
            .bind(g)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM supersessions WHERE graph_name = $1")
            .bind(g)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM properties WHERE graph_name = $1")
            .bind(g)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM source_credibility WHERE graph_name = $1")
            .bind(g)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // --- Entity search ---

    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> {
        // Try tsvector search first, fall back to ILIKE
        let tsquery = query_str.split_whitespace().collect::<Vec<_>>().join(" & ");

        let rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities
             WHERE graph_name = $1 AND to_tsvector('english', name) @@ to_tsquery('english', $2)",
        )
        .bind(&self.graph_name)
        .bind(&tsquery)
        .fetch_all(&self.pool)
        .await?;

        if !rows.is_empty() {
            return Ok(rows.iter().map(row_to_entity).collect());
        }

        // Fallback to ILIKE
        let pattern = format!("%{}%", query_str);
        let rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities
             WHERE graph_name = $1 AND name ILIKE $2",
        )
        .bind(&self.graph_name)
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_entity).collect())
    }

    async fn vector_search_entities(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EntityRow, f32)>> {
        let rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities WHERE graph_name = $1",
        )
        .bind(&self.graph_name)
        .fetch_all(&self.pool)
        .await?;

        let entities: Vec<EntityRow> = rows.iter().map(row_to_entity).collect();

        let mut scored: Vec<(EntityRow, f32)> = entities
            .into_iter()
            .filter(|e| !e.embedding.is_empty())
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (e, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        let row = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities WHERE graph_name = $1 AND id = $2",
        )
        .bind(&self.graph_name)
        .bind(entity_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.as_ref().map(row_to_entity))
    }

    // --- Edge search ---

    async fn fulltext_search_edges(
        &self,
        query_str: &str,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
        let tsquery = query_str.split_whitespace().collect::<Vec<_>>().join(" & ");

        let rows = if let Some(at) = at {
            let at_str = at.to_rfc3339();
            // Try tsvector first
            let ts_rows = sqlx::query(&format!(
                "{} WHERE e.graph_name = $1 AND e.valid_at <= $2 AND (e.invalid_at IS NULL OR e.invalid_at > $2)
                 AND to_tsvector('english', e.fact) @@ to_tsquery('english', $3)",
                EDGES_SELECT
            ))
            .bind(&self.graph_name)
            .bind(&at_str)
            .bind(&tsquery)
            .fetch_all(&self.pool)
            .await?;

            if !ts_rows.is_empty() {
                ts_rows
            } else {
                let pattern = format!("%{}%", query_str);
                sqlx::query(&format!(
                    "{} WHERE e.graph_name = $1 AND e.valid_at <= $2 AND (e.invalid_at IS NULL OR e.invalid_at > $2)
                     AND e.fact ILIKE $3",
                    EDGES_SELECT
                ))
                .bind(&self.graph_name)
                .bind(&at_str)
                .bind(&pattern)
                .fetch_all(&self.pool)
                .await?
            }
        } else {
            let ts_rows = sqlx::query(&format!(
                "{} WHERE e.graph_name = $1 AND e.invalid_at IS NULL
                 AND to_tsvector('english', e.fact) @@ to_tsquery('english', $2)",
                EDGES_SELECT
            ))
            .bind(&self.graph_name)
            .bind(&tsquery)
            .fetch_all(&self.pool)
            .await?;

            if !ts_rows.is_empty() {
                ts_rows
            } else {
                let pattern = format!("%{}%", query_str);
                sqlx::query(&format!(
                    "{} WHERE e.graph_name = $1 AND e.invalid_at IS NULL AND e.fact ILIKE $2",
                    EDGES_SELECT
                ))
                .bind(&self.graph_name)
                .bind(&pattern)
                .fetch_all(&self.pool)
                .await?
            }
        };

        Ok(rows.iter().map(row_to_edge).collect())
    }

    async fn vector_search_edges_scored(
        &self,
        embedding: &[f32],
        k: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, f32)>> {
        let rows = if let Some(at) = at {
            let at_str = at.to_rfc3339();
            sqlx::query(&format!(
                "{} WHERE e.graph_name = $1 AND e.valid_at <= $2 AND (e.invalid_at IS NULL OR e.invalid_at > $2)",
                EDGES_SELECT
            ))
            .bind(&self.graph_name)
            .bind(&at_str)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(&format!(
                "{} WHERE e.graph_name = $1 AND e.invalid_at IS NULL",
                EDGES_SELECT
            ))
            .bind(&self.graph_name)
            .fetch_all(&self.pool)
            .await?
        };

        let edges: Vec<EdgeRow> = rows.iter().map(row_to_edge).collect();

        let mut scored: Vec<(EdgeRow, f32)> = edges
            .into_iter()
            .filter(|e| !e.embedding.is_empty())
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (e, score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    // --- Graph traversal ---

    async fn walk_n_hops(
        &self,
        seed_entity_ids: &[String],
        max_hops: usize,
        limit_per_hop: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, usize)>> {
        if seed_entity_ids.is_empty() {
            return Ok(vec![]);
        }

        // Use recursive CTE for graph traversal
        let temporal_filter = if let Some(at) = &at {
            format!(
                "AND e.valid_at <= '{}' AND (e.invalid_at IS NULL OR e.invalid_at > '{}')",
                at.to_rfc3339(),
                at.to_rfc3339()
            )
        } else {
            "AND e.invalid_at IS NULL".to_string()
        };

        let cte_sql = format!(
            "WITH RECURSIVE hops AS (
                SELECT e.edge_id, e.from_id, e.to_id, 0 AS depth
                FROM edges e
                WHERE e.graph_name = $1
                  AND (e.from_id = ANY($2) OR e.to_id = ANY($2))
                  {temporal_filter}
                UNION ALL
                SELECT e.edge_id, e.from_id, e.to_id, h.depth + 1
                FROM edges e
                JOIN hops h ON (e.from_id = h.to_id OR e.to_id = h.from_id)
                WHERE e.graph_name = $1
                  {temporal_filter}
                  AND h.depth < $3
            )
            SELECT DISTINCT edge_id, depth + 1 AS hop FROM hops LIMIT $4"
        );

        let hop_rows = sqlx::query(&cte_sql)
            .bind(&self.graph_name)
            .bind(seed_entity_ids)
            .bind(max_hops as i64)
            .bind((limit_per_hop * max_hops) as i64)
            .fetch_all(&self.pool)
            .await?;

        let mut results = Vec::new();
        for hop_row in &hop_rows {
            let edge_id: i64 = hop_row.get("edge_id");
            let hop: i64 = hop_row.get("hop");

            // Fetch full edge row
            let edge_row = sqlx::query(&format!(
                "{} WHERE e.graph_name = $1 AND e.edge_id = $2",
                EDGES_SELECT
            ))
            .bind(&self.graph_name)
            .bind(edge_id)
            .fetch_optional(&self.pool)
            .await?;

            if let Some(row) = edge_row.as_ref() {
                results.push((row_to_edge(row), hop as usize));
            }
        }

        Ok(results)
    }

    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> {
        let sql = format!(
            "{} WHERE e.graph_name = $1 AND e.invalid_at IS NULL AND (e.from_id = $2 OR e.to_id = $2)",
            EDGES_SELECT
        );
        let rows = sqlx::query(&sql)
            .bind(&self.graph_name)
            .bind(node_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_edge).collect())
    }

    // --- Mutation ---

    async fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        let embedding_blob = serialize_embedding(&entity.embedding);
        sqlx::query(
            "INSERT INTO entities (graph_name, id, name, entity_type, resolved, hint, content, created_at, embedding)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (graph_name, id) DO UPDATE SET
                name = EXCLUDED.name,
                entity_type = EXCLUDED.entity_type,
                resolved = EXCLUDED.resolved,
                hint = EXCLUDED.hint,
                content = EXCLUDED.content,
                embedding = EXCLUDED.embedding",
        )
        .bind(&self.graph_name)
        .bind(&entity.id)
        .bind(&entity.name)
        .bind(&entity.entity_type)
        .bind(entity.resolved)
        .bind(&entity.hint)
        .bind(&entity.content)
        .bind(entity.created_at.to_rfc3339())
        .bind(&embedding_blob)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn create_edge(&self, from_id: &str, to_id: &str, rel: &Relation) -> Result<i64> {
        let embedding_blob = serialize_embedding(&rel.embedding);
        let source_agents = rel.source_agents.join(",");
        let memory_tier = match rel.memory_tier {
            MemoryTier::Working => "working",
            MemoryTier::LongTerm => "long_term",
        };

        let row = sqlx::query(
            "INSERT INTO edges (graph_name, from_id, to_id, fact, relation_type, confidence, salience,
             valid_at, invalid_at, embedding, source_agents, memory_tier, created_at, decayed_confidence, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
             RETURNING edge_id",
        )
        .bind(&self.graph_name)
        .bind(from_id)
        .bind(to_id)
        .bind(&rel.fact)
        .bind(&rel.relation_type)
        .bind(rel.confidence)
        .bind(rel.salience)
        .bind(rel.valid_at.to_rfc3339())
        .bind(rel.invalid_at.map(|t| t.to_rfc3339()))
        .bind(&embedding_blob)
        .bind(&source_agents)
        .bind(memory_tier)
        .bind(rel.created_at.to_rfc3339())
        .bind(rel.confidence)
        .bind(rel.expires_at.map(|t| t.to_rfc3339()))
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("edge_id"))
    }

    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE edges SET invalid_at = $1 WHERE graph_name = $2 AND edge_id = $3")
            .bind(at.to_rfc3339())
            .bind(&self.graph_name)
            .bind(edge_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> {
        sqlx::query("UPDATE edges SET from_id = $1 WHERE graph_name = $2 AND from_id = $3")
            .bind(resolved_id)
            .bind(&self.graph_name)
            .bind(placeholder_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE edges SET to_id = $1 WHERE graph_name = $2 AND to_id = $3")
            .bind(resolved_id)
            .bind(&self.graph_name)
            .bind(placeholder_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM entities WHERE graph_name = $1 AND id = $2")
            .bind(&self.graph_name)
            .bind(placeholder_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_entity(&self, entity_id: &str) -> Result<usize> {
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE edges SET invalid_at = $1
             WHERE graph_name = $2 AND (from_id = $3 OR to_id = $3) AND invalid_at IS NULL",
        )
        .bind(&now)
        .bind(&self.graph_name)
        .bind(entity_id)
        .execute(&self.pool)
        .await?;
        let count = result.rows_affected() as usize;

        sqlx::query("DELETE FROM entities WHERE graph_name = $1 AND id = $2")
            .bind(&self.graph_name)
            .bind(entity_id)
            .execute(&self.pool)
            .await?;
        Ok(count)
    }

    // --- Memory tier management ---

    async fn promote_working_memory(&self) -> Result<usize> {
        let threshold = (Utc::now() - Duration::hours(1)).to_rfc3339();
        let result = sqlx::query(
            "UPDATE edges SET memory_tier = 'long_term'
             WHERE graph_name = $1 AND memory_tier = 'working' AND invalid_at IS NULL
             AND salience >= 3 AND created_at < $2",
        )
        .bind(&self.graph_name)
        .bind(&threshold)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }

    async fn memory_tier_stats(&self) -> Result<crate::models::MemoryTierStats> {
        let working_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM edges WHERE graph_name = $1 AND invalid_at IS NULL AND memory_tier = 'working'",
        )
        .bind(&self.graph_name)
        .fetch_one(&self.pool)
        .await?;
        let working_count: i64 = working_row.get("cnt");

        let lt_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM edges WHERE graph_name = $1 AND invalid_at IS NULL AND memory_tier = 'long_term'",
        )
        .bind(&self.graph_name)
        .fetch_one(&self.pool)
        .await?;
        let long_term_count: i64 = lt_row.get("cnt");

        Ok(crate::models::MemoryTierStats {
            working_count: working_count as usize,
            long_term_count: long_term_count as usize,
        })
    }

    async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> Result<usize> {
        let threshold = stale_before.to_rfc3339();
        let result = sqlx::query(
            "UPDATE edges SET decayed_confidence = decayed_confidence * 0.95
             WHERE graph_name = $1 AND invalid_at IS NULL AND valid_at < $2",
        )
        .bind(&self.graph_name)
        .bind(&threshold)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }

    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> {
        let now_str = now.to_rfc3339();
        let result = sqlx::query(
            "UPDATE edges SET invalid_at = $1
             WHERE graph_name = $2 AND invalid_at IS NULL AND expires_at IS NOT NULL AND expires_at <= $1",
        )
        .bind(&now_str)
        .bind(&self.graph_name)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }

    // --- Facts / reflection ---

    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT fact FROM edges
             WHERE graph_name = $1 AND invalid_at IS NULL AND (from_id = $2 OR to_id = $2)",
        )
        .bind(&self.graph_name)
        .bind(entity_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(|r| r.get("fact")).collect())
    }

    async fn graph_stats(&self) -> Result<crate::models::GraphStats> {
        let entity_row = sqlx::query("SELECT COUNT(*) as cnt FROM entities WHERE graph_name = $1")
            .bind(&self.graph_name)
            .fetch_one(&self.pool)
            .await?;
        let entity_count: i64 = entity_row.get("cnt");

        let edge_row = sqlx::query(
            "SELECT COUNT(*) as cnt FROM edges WHERE graph_name = $1 AND invalid_at IS NULL",
        )
        .bind(&self.graph_name)
        .fetch_one(&self.pool)
        .await?;
        let edge_count: i64 = edge_row.get("cnt");

        let oldest_row = sqlx::query(
            "SELECT MIN(valid_at) as val FROM edges WHERE graph_name = $1 AND invalid_at IS NULL",
        )
        .bind(&self.graph_name)
        .fetch_one(&self.pool)
        .await?;
        let oldest_valid_at: Option<String> = oldest_row.get("val");

        let newest_row = sqlx::query(
            "SELECT MAX(valid_at) as val FROM edges WHERE graph_name = $1 AND invalid_at IS NULL",
        )
        .bind(&self.graph_name)
        .fetch_one(&self.pool)
        .await?;
        let newest_valid_at: Option<String> = newest_row.get("val");

        let avg_row = sqlx::query(
            "SELECT COALESCE(AVG(confidence), 0.0) as val FROM edges WHERE graph_name = $1 AND invalid_at IS NULL",
        )
        .bind(&self.graph_name)
        .fetch_one(&self.pool)
        .await?;
        let avg_confidence: f64 = avg_row.get("val");

        Ok(crate::models::GraphStats {
            entity_count: entity_count as usize,
            edge_count: edge_count as usize,
            oldest_valid_at,
            newest_valid_at,
            avg_confidence: avg_confidence as f32,
        })
    }

    // --- Dump / pagination ---

    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        let rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities WHERE graph_name = $1",
        )
        .bind(&self.graph_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_entity).collect())
    }

    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        let sql = format!("{} WHERE e.graph_name = $1", EDGES_SELECT);
        let rows = sqlx::query(&sql)
            .bind(&self.graph_name)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_edge).collect())
    }

    async fn list_entities_by_recency(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EntityRow>> {
        let rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities WHERE graph_name = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3",
        )
        .bind(&self.graph_name)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_entity).collect())
    }

    // --- Supersession / provenance ---

    async fn get_provenance(&self, edge_id: i64) -> Result<ProvenanceResponse> {
        let superseded_by: Option<SupersessionRecord> = sqlx::query(
            "SELECT old_edge_id, new_edge_id, superseded_at, old_fact, new_fact
             FROM supersessions WHERE graph_name = $1 AND old_edge_id = $2 LIMIT 1",
        )
        .bind(&self.graph_name)
        .bind(edge_id)
        .fetch_optional(&self.pool)
        .await?
        .map(|row| {
            let at_str: String = row.get("superseded_at");
            SupersessionRecord {
                old_edge_id: row.get("old_edge_id"),
                new_edge_id: row.get("new_edge_id"),
                superseded_at: at_str
                    .parse::<DateTime<Utc>>()
                    .unwrap_or_else(|_| Utc::now()),
                old_fact: row.get("old_fact"),
                new_fact: row.get("new_fact"),
            }
        });

        let supersedes_rows = sqlx::query(
            "SELECT old_edge_id, new_edge_id, superseded_at, old_fact, new_fact
             FROM supersessions WHERE graph_name = $1 AND new_edge_id = $2",
        )
        .bind(&self.graph_name)
        .bind(edge_id)
        .fetch_all(&self.pool)
        .await?;

        let supersedes: Vec<SupersessionRecord> = supersedes_rows
            .iter()
            .map(|row| {
                let at_str: String = row.get("superseded_at");
                SupersessionRecord {
                    old_edge_id: row.get("old_edge_id"),
                    new_edge_id: row.get("new_edge_id"),
                    superseded_at: at_str
                        .parse::<DateTime<Utc>>()
                        .unwrap_or_else(|_| Utc::now()),
                    old_fact: row.get("old_fact"),
                    new_fact: row.get("new_fact"),
                }
            })
            .collect();

        Ok(ProvenanceResponse {
            edge_id,
            superseded_by,
            supersedes,
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
        let linked_rows = sqlx::query(
            "SELECT DISTINCT from_id AS eid FROM edges WHERE graph_name = $1 AND invalid_at IS NULL AND (from_id = $2 OR to_id = $2)
             UNION
             SELECT DISTINCT to_id AS eid FROM edges WHERE graph_name = $1 AND invalid_at IS NULL AND (from_id = $2 OR to_id = $2)",
        )
        .bind(&self.graph_name)
        .bind(node_id)
        .fetch_all(&self.pool)
        .await?;

        let linked: std::collections::HashSet<String> =
            linked_rows.iter().map(|r| r.get("eid")).collect();

        let entity_rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities WHERE graph_name = $1",
        )
        .bind(&self.graph_name)
        .fetch_all(&self.pool)
        .await?;

        let entities: Vec<EntityRow> = entity_rows.iter().map(row_to_entity).collect();

        let mut results: Vec<(EntityRow, f32)> = entities
            .into_iter()
            .filter(|e| e.id != node_id && !linked.contains(&e.id) && !e.embedding.is_empty())
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (e, score)
            })
            .filter(|(_, score)| *score >= threshold)
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }

    async fn find_placeholder_nodes(&self, _cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> {
        let rows = sqlx::query(
            "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
             FROM entities WHERE graph_name = $1 AND resolved = FALSE",
        )
        .bind(&self.graph_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(row_to_entity).collect())
    }

    // --- Entity updates ---

    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        sqlx::query("UPDATE entities SET name = $1 WHERE graph_name = $2 AND id = $3")
            .bind(new_name)
            .bind(&self.graph_name)
            .bind(entity_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO properties (graph_name, entity_id, key, value) VALUES ($1, $2, $3, $4)
             ON CONFLICT (graph_name, entity_id, key) DO UPDATE SET value = EXCLUDED.value",
        )
        .bind(&self.graph_name)
        .bind(entity_id)
        .bind(key)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> {
        let prop_row = sqlx::query(
            "SELECT entity_id FROM properties WHERE graph_name = $1 AND key = $2 AND value = $3 LIMIT 1",
        )
        .bind(&self.graph_name)
        .bind(key)
        .bind(value)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(prop) = prop_row {
            let eid: String = prop.get("entity_id");
            let row = sqlx::query(
                "SELECT id, name, entity_type, resolved, hint, content, created_at, embedding
                 FROM entities WHERE graph_name = $1 AND id = $2",
            )
            .bind(&self.graph_name)
            .bind(&eid)
            .fetch_optional(&self.pool)
            .await?;
            Ok(row.as_ref().map(row_to_entity))
        } else {
            Ok(None)
        }
    }
}

// --- Extra methods (mirror SqliteGraph non-trait methods) ---

impl PostgresGraph {
    pub async fn save_source_credibility(
        &self,
        cred: &crate::credibility::SourceCredibility,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO source_credibility (graph_name, agent_id, credibility, fact_count, contradiction_rate)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (graph_name, agent_id) DO UPDATE SET
                credibility = EXCLUDED.credibility,
                fact_count = EXCLUDED.fact_count,
                contradiction_rate = EXCLUDED.contradiction_rate",
        )
        .bind(&self.graph_name)
        .bind(&cred.agent_id)
        .bind(cred.credibility)
        .bind(cred.fact_count as i64)
        .bind(cred.contradiction_rate)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_all_source_credibility(
        &self,
    ) -> Result<Vec<crate::credibility::SourceCredibility>> {
        let rows = sqlx::query(
            "SELECT agent_id, credibility, fact_count, contradiction_rate
             FROM source_credibility WHERE graph_name = $1",
        )
        .bind(&self.graph_name)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| crate::credibility::SourceCredibility {
                agent_id: row.get("agent_id"),
                credibility: row.get("credibility"),
                fact_count: row.get::<i64, _>("fact_count") as usize,
                contradiction_rate: row.get("contradiction_rate"),
            })
            .collect())
    }

    pub async fn compound_edge_confidence(
        &self,
        edge_id: i64,
        new_agent: &str,
        new_confidence: f32,
    ) -> Result<f32> {
        let row = sqlx::query(
            "SELECT confidence, source_agents FROM edges WHERE graph_name = $1 AND edge_id = $2",
        )
        .bind(&self.graph_name)
        .bind(edge_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let old_conf: f32 = row.get("confidence");
            let agents_str: String = row.get("source_agents");
            let combined = 1.0 - (1.0 - old_conf) * (1.0 - new_confidence);
            let new_agents = if agents_str.split(',').any(|a| a == new_agent) {
                agents_str
            } else if agents_str.is_empty() {
                new_agent.to_string()
            } else {
                format!("{},{}", agents_str, new_agent)
            };
            sqlx::query(
                "UPDATE edges SET confidence = $1, decayed_confidence = $1, source_agents = $2
                 WHERE graph_name = $3 AND edge_id = $4",
            )
            .bind(combined)
            .bind(&new_agents)
            .bind(&self.graph_name)
            .bind(edge_id)
            .execute(&self.pool)
            .await?;
            Ok(combined)
        } else {
            Ok(new_confidence)
        }
    }

    pub async fn create_supersession(
        &self,
        old_edge_id: i64,
        new_edge_id: i64,
        superseded_at: DateTime<Utc>,
        old_fact: &str,
        new_fact: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO supersessions (graph_name, old_edge_id, new_edge_id, superseded_at, old_fact, new_fact)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&self.graph_name)
        .bind(old_edge_id)
        .bind(new_edge_id)
        .bind(superseded_at.to_rfc3339())
        .bind(old_fact)
        .bind(new_fact)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_supersession_chain(&self, edge_id: i64) -> Result<Vec<SupersessionRecord>> {
        let rows = sqlx::query(
            "SELECT old_edge_id, new_edge_id, superseded_at, old_fact, new_fact
             FROM supersessions WHERE graph_name = $1 AND (old_edge_id = $2 OR new_edge_id = $2)",
        )
        .bind(&self.graph_name)
        .bind(edge_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|row| {
                let at_str: String = row.get("superseded_at");
                SupersessionRecord {
                    old_edge_id: row.get("old_edge_id"),
                    new_edge_id: row.get("new_edge_id"),
                    superseded_at: at_str
                        .parse::<DateTime<Utc>>()
                        .unwrap_or_else(|_| Utc::now()),
                    old_fact: row.get("old_fact"),
                    new_fact: row.get("new_fact"),
                }
            })
            .collect())
    }
}
