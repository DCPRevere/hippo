use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};

use crate::error::GraphConnectError;
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use qdrant_client::qdrant::{
    value::Kind, with_payload_selector, with_vectors_selector, Condition, CreateCollectionBuilder,
    DeleteCollectionBuilder, DeletePointsBuilder, Distance, Filter, PointId, PointStruct,
    PointsIdsList, ScrollPointsBuilder, SearchPointsBuilder, SetPayloadPointsBuilder,
    UpsertPointsBuilder, VectorParamsBuilder,
};
use qdrant_client::Qdrant;

use crate::graph_backend::GraphBackend;
use crate::models::{
    EdgeRow, Entity, EntityRow, GraphStats, MemoryTier, MemoryTierStats, ProvenanceResponse,
    Relation,
};

/// Hash a string ID to a u64 for use as a Qdrant point ID.
fn hash_id(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

/// Hash an i64 edge ID to a u64 point ID.
fn edge_point_id(edge_id: i64) -> u64 {
    edge_id as u64
}

fn payload_str(payload: &HashMap<String, qdrant_client::qdrant::Value>, key: &str) -> String {
    payload
        .get(key)
        .and_then(|v| match &v.kind {
            Some(Kind::StringValue(s)) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn payload_str_opt(
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
    key: &str,
) -> Option<String> {
    payload.get(key).and_then(|v| match &v.kind {
        Some(Kind::StringValue(s)) => {
            if s.is_empty() {
                None
            } else {
                Some(s.clone())
            }
        }
        Some(Kind::NullValue(_)) => None,
        _ => None,
    })
}

fn payload_f64(payload: &HashMap<String, qdrant_client::qdrant::Value>, key: &str) -> f64 {
    payload
        .get(key)
        .and_then(|v| match &v.kind {
            Some(Kind::DoubleValue(d)) => Some(*d),
            Some(Kind::IntegerValue(i)) => Some(*i as f64),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn payload_i64(payload: &HashMap<String, qdrant_client::qdrant::Value>, key: &str) -> i64 {
    payload
        .get(key)
        .and_then(|v| match &v.kind {
            Some(Kind::IntegerValue(i)) => Some(*i),
            Some(Kind::DoubleValue(d)) => Some(*d as i64),
            _ => None,
        })
        .unwrap_or(0)
}

fn payload_bool(payload: &HashMap<String, qdrant_client::qdrant::Value>, key: &str) -> bool {
    payload
        .get(key)
        .and_then(|v| match &v.kind {
            Some(Kind::BoolValue(b)) => Some(*b),
            _ => None,
        })
        .unwrap_or(false)
}

fn entity_row_from_payload(
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
    embedding: Vec<f32>,
) -> EntityRow {
    EntityRow {
        id: payload_str(payload, "id"),
        name: payload_str(payload, "name"),
        entity_type: payload_str(payload, "entity_type"),
        resolved: payload_bool(payload, "resolved"),
        hint: payload_str_opt(payload, "hint"),
        content: payload_str_opt(payload, "content"),
        created_at: payload_str(payload, "created_at"),
        embedding,
    }
}

fn edge_row_from_payload(
    payload: &HashMap<String, qdrant_client::qdrant::Value>,
    embedding: Vec<f32>,
) -> EdgeRow {
    EdgeRow {
        edge_id: payload_i64(payload, "edge_id"),
        subject_id: payload_str(payload, "from_id"),
        subject_name: payload_str(payload, "from_name"),
        fact: payload_str(payload, "fact_text"),
        relation_type: payload_str(payload, "relation_type"),
        confidence: payload_f64(payload, "confidence") as f32,
        salience: payload_i64(payload, "salience"),
        valid_at: payload_str(payload, "valid_at"),
        invalid_at: payload_str_opt(payload, "invalid_at"),
        object_id: payload_str(payload, "to_id"),
        object_name: payload_str(payload, "to_name"),
        embedding,
        decayed_confidence: payload_f64(payload, "decayed_confidence") as f32,
        source_agents: payload_str(payload, "source_agents"),
        memory_tier: payload_str(payload, "tier"),
        expires_at: payload_str_opt(payload, "expires_at"),
    }
}

fn qdrant_value_str(s: &str) -> qdrant_client::qdrant::Value {
    qdrant_client::qdrant::Value {
        kind: Some(Kind::StringValue(s.to_string())),
    }
}

fn qdrant_value_str_opt(s: &Option<String>) -> qdrant_client::qdrant::Value {
    match s {
        Some(v) => qdrant_value_str(v),
        None => qdrant_client::qdrant::Value {
            kind: Some(Kind::NullValue(0)),
        },
    }
}

fn qdrant_value_bool(b: bool) -> qdrant_client::qdrant::Value {
    qdrant_client::qdrant::Value {
        kind: Some(Kind::BoolValue(b)),
    }
}

fn qdrant_value_f64(f: f64) -> qdrant_client::qdrant::Value {
    qdrant_client::qdrant::Value {
        kind: Some(Kind::DoubleValue(f)),
    }
}

fn qdrant_value_i64(i: i64) -> qdrant_client::qdrant::Value {
    qdrant_client::qdrant::Value {
        kind: Some(Kind::IntegerValue(i)),
    }
}

pub struct QdrantGraph {
    client: Qdrant,
    graph_name: String,
    next_edge_id: AtomicI64,
}

impl QdrantGraph {
    pub async fn new(url: &str, graph_name: &str) -> Result<Self> {
        let client = Qdrant::from_url(url)
            .build()
            .map_err(|e| GraphConnectError::new(format!("failed to connect to Qdrant: {e}")))?;

        let graph = Self {
            client,
            graph_name: graph_name.to_string(),
            next_edge_id: AtomicI64::new(1),
        };

        // Initialize next_edge_id from existing edges
        graph.init_next_edge_id().await?;

        Ok(graph)
    }

    fn entities_collection(&self) -> String {
        format!("{}_entities", self.graph_name)
    }

    fn edges_collection(&self) -> String {
        format!("{}_edges", self.graph_name)
    }

    async fn init_next_edge_id(&self) -> Result<()> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(());
        }

        // Scroll through all edge points to find the max edge_id
        let mut max_id: i64 = 0;
        let mut offset: Option<PointId> = None;
        loop {
            let mut builder = ScrollPointsBuilder::new(&collection)
                .limit(100)
                .with_payload(with_payload_selector::SelectorOptions::from(true))
                .with_vectors(with_vectors_selector::SelectorOptions::from(false));
            if let Some(ref o) = offset {
                builder = builder.offset(o.clone());
            }
            let result = self.client.scroll(builder).await?;
            if result.result.is_empty() {
                break;
            }
            for point in &result.result {
                let eid = payload_i64(&point.payload, "edge_id");
                if eid > max_id {
                    max_id = eid;
                }
            }
            match result.next_page_offset {
                Some(o) => offset = Some(o),
                None => break,
            }
        }
        self.next_edge_id.store(max_id + 1, Ordering::Relaxed);
        Ok(())
    }

    async fn collection_exists(&self, name: &str) -> Result<bool> {
        let collections = self.client.list_collections().await?;
        Ok(collections.collections.iter().any(|c| c.name == name))
    }

    async fn ensure_collection(&self, name: &str, vector_size: u64) -> Result<()> {
        if self.collection_exists(name).await? {
            return Ok(());
        }
        self.client
            .create_collection(
                CreateCollectionBuilder::new(name)
                    .vectors_config(VectorParamsBuilder::new(vector_size, Distance::Cosine)),
            )
            .await
            .with_context(|| format!("failed to create collection {name}"))?;
        Ok(())
    }

    /// Scroll all points from a collection matching the given filter.
    async fn scroll_all(
        &self,
        collection: &str,
        filter: Option<Filter>,
        with_vectors: bool,
    ) -> Result<Vec<qdrant_client::qdrant::RetrievedPoint>> {
        let mut all_points = Vec::new();
        let mut offset: Option<PointId> = None;
        loop {
            let mut builder = ScrollPointsBuilder::new(collection)
                .limit(100)
                .with_payload(with_payload_selector::SelectorOptions::from(true))
                .with_vectors(with_vectors_selector::SelectorOptions::from(with_vectors));
            if let Some(ref f) = filter {
                builder = builder.filter(f.clone());
            }
            if let Some(ref o) = offset {
                builder = builder.offset(o.clone());
            }
            let result = self.client.scroll(builder).await?;
            all_points.extend(result.result);
            match result.next_page_offset {
                Some(o) => offset = Some(o),
                None => break,
            }
        }
        Ok(all_points)
    }

    fn extract_vectors(point: &qdrant_client::qdrant::RetrievedPoint) -> Vec<f32> {
        use qdrant_client::qdrant::vector_output::Vector;
        point
            .vectors
            .as_ref()
            .and_then(|vs| vs.get_vector())
            .and_then(|v| match v {
                Vector::Dense(dv) => Some(dv.data),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Look up entity names for a set of entity IDs. Returns id->name map.
    async fn entity_names(&self, entity_ids: &[String]) -> Result<HashMap<String, String>> {
        if entity_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(HashMap::new());
        }

        let mut map = HashMap::new();
        // Batch lookup via scroll with filter on id field
        for chunk in entity_ids.chunks(50) {
            let conditions: Vec<Condition> = chunk
                .iter()
                .map(|id| Condition::matches("id", id.as_str().to_string()))
                .collect();
            let filter = Filter::should(conditions);
            let points = self.scroll_all(&collection, Some(filter), false).await?;
            for point in &points {
                let id = payload_str(&point.payload, "id");
                let name = payload_str(&point.payload, "name");
                map.insert(id, name);
            }
        }
        Ok(map)
    }

    /// Enrich edge rows with entity names.
    async fn enrich_edge_rows(&self, edges: &mut [EdgeRow]) -> Result<()> {
        let mut ids: HashSet<String> = HashSet::new();
        for e in edges.iter() {
            ids.insert(e.subject_id.clone());
            ids.insert(e.object_id.clone());
        }
        let id_vec: Vec<String> = ids.into_iter().collect();
        let names = self.entity_names(&id_vec).await?;
        for e in edges.iter_mut() {
            if let Some(name) = names.get(&e.subject_id) {
                e.subject_name = name.clone();
            }
            if let Some(name) = names.get(&e.object_id) {
                e.object_name = name.clone();
            }
        }
        Ok(())
    }
}

#[async_trait]
impl GraphBackend for QdrantGraph {
    fn graph_name(&self) -> &str {
        &self.graph_name
    }

    async fn ping(&self) -> Result<()> {
        self.client.health_check().await?;
        Ok(())
    }

    async fn setup_schema(&self) -> Result<()> {
        // Use a default vector size; collections will be created lazily on first upsert
        // with the actual embedding dimension.
        Ok(())
    }

    async fn drop_and_reinitialise(&self) -> Result<()> {
        let entities_col = self.entities_collection();
        let edges_col = self.edges_collection();
        if self.collection_exists(&entities_col).await? {
            self.client
                .delete_collection(DeleteCollectionBuilder::new(&entities_col))
                .await?;
        }
        if self.collection_exists(&edges_col).await? {
            self.client
                .delete_collection(DeleteCollectionBuilder::new(&edges_col))
                .await?;
        }
        self.next_edge_id.store(1, Ordering::Relaxed);
        Ok(())
    }

    // --- Entity search ---

    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        // Qdrant doesn't have true fulltext; scroll all and filter in Rust
        let points = self.scroll_all(&collection, None, true).await?;
        let lower = query_str.to_lowercase();
        let mut results = Vec::new();
        for point in &points {
            let name = payload_str(&point.payload, "name").to_lowercase();
            if name.contains(&lower) {
                let embedding = Self::extract_vectors(point);
                results.push(entity_row_from_payload(&point.payload, embedding));
            }
        }
        Ok(results)
    }

    async fn vector_search_entities(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EntityRow, f32)>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let results = self
            .client
            .search_points(
                SearchPointsBuilder::new(&collection, embedding.to_vec(), k as u64)
                    .with_payload(true)
                    .with_vectors(true),
            )
            .await?;

        Ok(results
            .result
            .iter()
            .map(|r| {
                let emb = r
                    .vectors
                    .as_ref()
                    .and_then(|vs| vs.get_vector())
                    .and_then(|v| {
                        use qdrant_client::qdrant::vector_output::Vector;
                        match v {
                            Vector::Dense(dv) => Some(dv.data),
                            _ => None,
                        }
                    })
                    .unwrap_or_default();
                (entity_row_from_payload(&r.payload, emb), r.score)
            })
            .collect())
    }

    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(None);
        }

        let filter = Filter::must(vec![Condition::matches("id", entity_id.to_string())]);
        let points = self.scroll_all(&collection, Some(filter), true).await?;
        Ok(points.first().map(|p| {
            let emb = Self::extract_vectors(p);
            entity_row_from_payload(&p.payload, emb)
        }))
    }

    // --- Edge search ---

    async fn fulltext_search_edges(
        &self,
        query_str: &str,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let points = self.scroll_all(&collection, None, true).await?;
        let lower = query_str.to_lowercase();
        let mut results = Vec::new();
        for point in &points {
            let fact = payload_str(&point.payload, "fact_text").to_lowercase();
            if !fact.contains(&lower) {
                continue;
            }
            let emb = Self::extract_vectors(point);
            let row = edge_row_from_payload(&point.payload, emb);
            // Check temporal constraint
            if let Some(at) = at {
                let at_str = at.to_rfc3339();
                if row.valid_at > at_str {
                    continue;
                }
                if let Some(ref inv) = row.invalid_at {
                    if inv <= &at_str {
                        continue;
                    }
                }
            } else {
                // Only active edges
                if row.invalid_at.is_some() {
                    continue;
                }
            }
            results.push(row);
        }
        self.enrich_edge_rows(&mut results).await?;
        Ok(results)
    }

    async fn vector_search_edges_scored(
        &self,
        embedding: &[f32],
        k: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, f32)>> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        // Build filter for active edges
        let filter = if at.is_some() {
            // Can't do complex temporal with Qdrant filters alone; fetch more and filter in Rust
            None
        } else {
            Some(Filter::must(vec![Condition::is_null("invalid_at")]))
        };

        let mut builder = SearchPointsBuilder::new(&collection, embedding.to_vec(), (k * 3) as u64)
            .with_payload(true)
            .with_vectors(true);
        if let Some(f) = filter {
            builder = builder.filter(f);
        }

        let results = self.client.search_points(builder).await?;

        let mut scored: Vec<(EdgeRow, f32)> = Vec::new();
        for r in &results.result {
            let emb = r
                .vectors
                .as_ref()
                .and_then(|vs| vs.get_vector())
                .and_then(|v| {
                    use qdrant_client::qdrant::vector_output::Vector;
                    match v {
                        Vector::Dense(dv) => Some(dv.data),
                        _ => None,
                    }
                })
                .unwrap_or_default();
            let row = edge_row_from_payload(&r.payload, emb);

            // Apply temporal filter
            if let Some(at) = at {
                let at_str = at.to_rfc3339();
                if row.valid_at > at_str {
                    continue;
                }
                if let Some(ref inv) = row.invalid_at {
                    if inv <= &at_str {
                        continue;
                    }
                }
            } else if row.invalid_at.is_some() {
                continue;
            }

            scored.push((row, r.score));
        }
        scored.truncate(k);

        // Enrich with entity names
        let mut rows: Vec<EdgeRow> = scored.iter().map(|(r, _)| r.clone()).collect();
        self.enrich_edge_rows(&mut rows).await?;
        Ok(rows
            .into_iter()
            .zip(scored.iter().map(|(_, s)| *s))
            .collect())
    }

    // --- Graph traversal ---

    async fn walk_n_hops(
        &self,
        seed_entity_ids: &[String],
        max_hops: usize,
        limit_per_hop: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, usize)>> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let mut results: Vec<(EdgeRow, usize)> = Vec::new();
        let mut frontier: Vec<String> = seed_entity_ids.to_vec();
        let mut visited_edges: HashSet<i64> = HashSet::new();

        for hop in 1..=max_hops {
            if frontier.is_empty() {
                break;
            }

            // Fetch edges where from_id or to_id is in the frontier
            let from_conditions: Vec<Condition> = frontier
                .iter()
                .map(|id| Condition::matches("from_id", id.as_str().to_string()))
                .collect();
            let to_conditions: Vec<Condition> = frontier
                .iter()
                .map(|id| Condition::matches("to_id", id.as_str().to_string()))
                .collect();

            let filter = Filter::should(
                from_conditions
                    .into_iter()
                    .chain(to_conditions)
                    .collect::<Vec<_>>(),
            );

            let points = self.scroll_all(&collection, Some(filter), true).await?;

            let mut next_frontier = Vec::new();
            let mut hop_count = 0;
            for point in &points {
                if hop_count >= limit_per_hop {
                    break;
                }
                let emb = Self::extract_vectors(point);
                let row = edge_row_from_payload(&point.payload, emb);

                // Temporal filter
                if let Some(at) = at {
                    let at_str = at.to_rfc3339();
                    if row.valid_at > at_str {
                        continue;
                    }
                    if let Some(ref inv) = row.invalid_at {
                        if inv <= &at_str {
                            continue;
                        }
                    }
                } else if row.invalid_at.is_some() {
                    continue;
                }

                if visited_edges.insert(row.edge_id) {
                    if !frontier.contains(&row.subject_id) {
                        next_frontier.push(row.subject_id.clone());
                    }
                    if !frontier.contains(&row.object_id) {
                        next_frontier.push(row.object_id.clone());
                    }
                    results.push((row, hop));
                    hop_count += 1;
                }
            }
            frontier = next_frontier;
        }

        // Enrich with names
        let mut rows: Vec<EdgeRow> = results.iter().map(|(r, _)| r.clone()).collect();
        self.enrich_edge_rows(&mut rows).await?;
        Ok(rows
            .into_iter()
            .zip(results.iter().map(|(_, h)| *h))
            .collect())
    }

    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        // (invalid_at IS NULL) AND (from_id = node_id OR to_id = node_id)
        let filter = Filter::must(vec![
            Condition::is_null("invalid_at"),
            Condition::from(Filter::should(vec![
                Condition::matches("from_id", node_id.to_string()),
                Condition::matches("to_id", node_id.to_string()),
            ])),
        ]);

        let points = self.scroll_all(&collection, Some(filter), true).await?;
        let mut results: Vec<EdgeRow> = points
            .iter()
            .map(|p| {
                let emb = Self::extract_vectors(p);
                edge_row_from_payload(&p.payload, emb)
            })
            .collect();
        self.enrich_edge_rows(&mut results).await?;
        Ok(results)
    }

    // --- Mutation ---

    async fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        let collection = self.entities_collection();
        let dim = entity.embedding.len() as u64;
        if dim == 0 {
            anyhow::bail!("entity embedding is empty");
        }
        self.ensure_collection(&collection, dim).await?;

        let point_id = hash_id(&entity.id);
        let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
        payload.insert("id".into(), qdrant_value_str(&entity.id));
        payload.insert("name".into(), qdrant_value_str(&entity.name));
        payload.insert("entity_type".into(), qdrant_value_str(&entity.entity_type));
        payload.insert("resolved".into(), qdrant_value_bool(entity.resolved));
        payload.insert("hint".into(), qdrant_value_str_opt(&entity.hint));
        payload.insert("content".into(), qdrant_value_str_opt(&entity.content));
        payload.insert(
            "created_at".into(),
            qdrant_value_str(&entity.created_at.to_rfc3339()),
        );

        let point = PointStruct::new(point_id, entity.embedding.clone(), payload);
        self.client
            .upsert_points(UpsertPointsBuilder::new(&collection, vec![point]).wait(true))
            .await?;
        Ok(())
    }

    async fn create_edge(&self, from_id: &str, to_id: &str, rel: &Relation) -> Result<i64> {
        let collection = self.edges_collection();
        let dim = rel.embedding.len() as u64;
        if dim == 0 {
            anyhow::bail!("edge embedding is empty");
        }
        self.ensure_collection(&collection, dim).await?;

        let edge_id = self.next_edge_id.fetch_add(1, Ordering::Relaxed);
        let point_id = edge_point_id(edge_id);

        // Look up entity names
        let names = self
            .entity_names(&[from_id.to_string(), to_id.to_string()])
            .await?;
        let from_name = names.get(from_id).cloned().unwrap_or_default();
        let to_name = names.get(to_id).cloned().unwrap_or_default();

        let tier = match rel.memory_tier {
            MemoryTier::Working => "working",
            MemoryTier::LongTerm => "long_term",
        };

        let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
        payload.insert("edge_id".into(), qdrant_value_i64(edge_id));
        payload.insert("from_id".into(), qdrant_value_str(from_id));
        payload.insert("to_id".into(), qdrant_value_str(to_id));
        payload.insert("from_name".into(), qdrant_value_str(&from_name));
        payload.insert("to_name".into(), qdrant_value_str(&to_name));
        payload.insert("relation_type".into(), qdrant_value_str(&rel.relation_type));
        payload.insert("fact_text".into(), qdrant_value_str(&rel.fact));
        payload.insert("confidence".into(), qdrant_value_f64(rel.confidence as f64));
        payload.insert("salience".into(), qdrant_value_i64(rel.salience));
        payload.insert(
            "valid_at".into(),
            qdrant_value_str(&rel.valid_at.to_rfc3339()),
        );
        payload.insert(
            "invalid_at".into(),
            qdrant_value_str_opt(&rel.invalid_at.map(|t| t.to_rfc3339())),
        );
        payload.insert("tier".into(), qdrant_value_str(tier));
        payload.insert(
            "source_agents".into(),
            qdrant_value_str(&rel.source_agents.join(",")),
        );
        payload.insert(
            "decayed_confidence".into(),
            qdrant_value_f64(rel.confidence as f64),
        );
        payload.insert(
            "expires_at".into(),
            qdrant_value_str_opt(&rel.expires_at.map(|t| t.to_rfc3339())),
        );
        payload.insert(
            "created_at".into(),
            qdrant_value_str(&rel.created_at.to_rfc3339()),
        );

        let point = PointStruct::new(point_id, rel.embedding.clone(), payload);
        self.client
            .upsert_points(UpsertPointsBuilder::new(&collection, vec![point]).wait(true))
            .await?;
        Ok(edge_id)
    }

    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(());
        }

        let point_id = edge_point_id(edge_id);
        let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
        payload.insert("invalid_at".into(), qdrant_value_str(&at.to_rfc3339()));

        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(&collection, payload)
                    .points_selector(PointsIdsList::from(vec![PointId::from(point_id)]))
                    .wait(true),
            )
            .await?;
        Ok(())
    }

    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> {
        let edges_col = self.edges_collection();
        if self.collection_exists(&edges_col).await? {
            // Update edges referencing the placeholder
            let filter = Filter::should(vec![
                Condition::matches("from_id", placeholder_id.to_string()),
                Condition::matches("to_id", placeholder_id.to_string()),
            ]);
            let points = self.scroll_all(&edges_col, Some(filter), false).await?;

            for point in &points {
                let from_id = payload_str(&point.payload, "from_id");
                let to_id = payload_str(&point.payload, "to_id");
                let mut update: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
                if from_id == placeholder_id {
                    update.insert("from_id".into(), qdrant_value_str(resolved_id));
                }
                if to_id == placeholder_id {
                    update.insert("to_id".into(), qdrant_value_str(resolved_id));
                }
                if !update.is_empty() {
                    if let Some(pid) = &point.id {
                        self.client
                            .set_payload(
                                SetPayloadPointsBuilder::new(&edges_col, update)
                                    .points_selector(vec![pid.clone()])
                                    .wait(true),
                            )
                            .await?;
                    }
                }
            }
        }

        // Delete the placeholder entity
        let entities_col = self.entities_collection();
        if self.collection_exists(&entities_col).await? {
            let point_id = hash_id(placeholder_id);
            self.client
                .delete_points(
                    DeletePointsBuilder::new(&entities_col)
                        .points(PointsIdsList {
                            ids: vec![point_id.into()],
                        })
                        .wait(true),
                )
                .await?;
        }
        Ok(())
    }

    async fn delete_entity(&self, entity_id: &str) -> Result<usize> {
        let now = Utc::now();
        let mut count = 0;

        // Invalidate all active edges involving this entity
        let edges_col = self.edges_collection();
        if self.collection_exists(&edges_col).await? {
            let filter = Filter::must(vec![
                Condition::is_null("invalid_at"),
                Condition::from(Filter::should(vec![
                    Condition::matches("from_id", entity_id.to_string()),
                    Condition::matches("to_id", entity_id.to_string()),
                ])),
            ]);
            let points = self.scroll_all(&edges_col, Some(filter), false).await?;
            count = points.len();

            for point in &points {
                if let Some(pid) = &point.id {
                    let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
                    payload.insert("invalid_at".into(), qdrant_value_str(&now.to_rfc3339()));
                    self.client
                        .set_payload(
                            SetPayloadPointsBuilder::new(&edges_col, payload)
                                .points_selector(vec![pid.clone()])
                                .wait(true),
                        )
                        .await?;
                }
            }
        }

        // Delete the entity
        let entities_col = self.entities_collection();
        if self.collection_exists(&entities_col).await? {
            let point_id = hash_id(entity_id);
            self.client
                .delete_points(
                    DeletePointsBuilder::new(&entities_col)
                        .points(PointsIdsList {
                            ids: vec![point_id.into()],
                        })
                        .wait(true),
                )
                .await?;
        }

        Ok(count)
    }

    // --- Memory tier management ---

    async fn promote_working_memory(&self) -> Result<usize> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(0);
        }

        let filter = Filter::must(vec![
            Condition::matches("tier", "working".to_string()),
            Condition::is_null("invalid_at"),
        ]);
        let points = self.scroll_all(&collection, Some(filter), false).await?;

        let threshold = Utc::now() - Duration::hours(1);
        let threshold_str = threshold.to_rfc3339();
        let mut count = 0;

        for point in &points {
            let salience = payload_i64(&point.payload, "salience");
            let created_at = payload_str(&point.payload, "created_at");
            if salience >= 3 && created_at < threshold_str {
                if let Some(pid) = &point.id {
                    let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
                    payload.insert("tier".into(), qdrant_value_str("long_term"));
                    self.client
                        .set_payload(
                            SetPayloadPointsBuilder::new(&collection, payload)
                                .points_selector(vec![pid.clone()])
                                .wait(true),
                        )
                        .await?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    async fn memory_tier_stats(&self) -> Result<MemoryTierStats> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(MemoryTierStats {
                working_count: 0,
                long_term_count: 0,
            });
        }

        let active_filter = Filter::must(vec![Condition::is_null("invalid_at")]);
        let points = self
            .scroll_all(&collection, Some(active_filter), false)
            .await?;

        let mut working = 0;
        let mut long_term = 0;
        for point in &points {
            let tier = payload_str(&point.payload, "tier");
            match tier.as_str() {
                "working" => working += 1,
                "long_term" => long_term += 1,
                _ => {}
            }
        }
        Ok(MemoryTierStats {
            working_count: working,
            long_term_count: long_term,
        })
    }

    async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> Result<usize> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(0);
        }

        let active_filter = Filter::must(vec![Condition::is_null("invalid_at")]);
        let points = self
            .scroll_all(&collection, Some(active_filter), false)
            .await?;

        let stale_str = stale_before.to_rfc3339();
        let mut count = 0;
        for point in &points {
            let valid_at = payload_str(&point.payload, "valid_at");
            if valid_at < stale_str {
                if let Some(pid) = &point.id {
                    let current_confidence = payload_f64(&point.payload, "decayed_confidence");
                    let new_confidence = current_confidence * 0.95;
                    let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
                    payload.insert(
                        "decayed_confidence".into(),
                        qdrant_value_f64(new_confidence),
                    );
                    self.client
                        .set_payload(
                            SetPayloadPointsBuilder::new(&collection, payload)
                                .points_selector(vec![pid.clone()])
                                .wait(true),
                        )
                        .await?;
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(0);
        }

        let active_filter = Filter::must(vec![Condition::is_null("invalid_at")]);
        let points = self
            .scroll_all(&collection, Some(active_filter), false)
            .await?;

        let now_str = now.to_rfc3339();
        let mut count = 0;
        for point in &points {
            if let Some(expires_at) = payload_str_opt(&point.payload, "expires_at") {
                if expires_at <= now_str {
                    if let Some(pid) = &point.id {
                        let mut payload: HashMap<String, qdrant_client::qdrant::Value> =
                            HashMap::new();
                        payload.insert("invalid_at".into(), qdrant_value_str(&now_str));
                        self.client
                            .set_payload(
                                SetPayloadPointsBuilder::new(&collection, payload)
                                    .points_selector(vec![pid.clone()])
                                    .wait(true),
                            )
                            .await?;
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    // --- Facts ---

    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let filter = Filter::must(vec![
            Condition::is_null("invalid_at"),
            Condition::from(Filter::should(vec![
                Condition::matches("from_id", entity_id.to_string()),
                Condition::matches("to_id", entity_id.to_string()),
            ])),
        ]);

        let points = self.scroll_all(&collection, Some(filter), false).await?;
        Ok(points
            .iter()
            .map(|p| payload_str(&p.payload, "fact_text"))
            .collect())
    }

    async fn graph_stats(&self) -> Result<GraphStats> {
        let entities_col = self.entities_collection();
        let edges_col = self.edges_collection();

        let entity_count = if self.collection_exists(&entities_col).await? {
            let points = self.scroll_all(&entities_col, None, false).await?;
            points.len()
        } else {
            0
        };

        if !self.collection_exists(&edges_col).await? {
            return Ok(GraphStats {
                entity_count,
                edge_count: 0,
                oldest_valid_at: None,
                newest_valid_at: None,
                avg_confidence: 0.0,
            });
        }

        let active_filter = Filter::must(vec![Condition::is_null("invalid_at")]);
        let points = self
            .scroll_all(&edges_col, Some(active_filter), false)
            .await?;

        let edge_count = points.len();
        let mut sum_confidence = 0.0f32;
        let mut oldest: Option<String> = None;
        let mut newest: Option<String> = None;

        for p in &points {
            sum_confidence += payload_f64(&p.payload, "confidence") as f32;
            let valid_at = payload_str(&p.payload, "valid_at");
            match &oldest {
                None => oldest = Some(valid_at.clone()),
                Some(o) if valid_at < *o => oldest = Some(valid_at.clone()),
                _ => {}
            }
            match &newest {
                None => newest = Some(valid_at.clone()),
                Some(n) if valid_at > *n => newest = Some(valid_at.clone()),
                _ => {}
            }
        }

        let avg_confidence = if edge_count > 0 {
            sum_confidence / edge_count as f32
        } else {
            0.0
        };

        Ok(GraphStats {
            entity_count,
            edge_count,
            oldest_valid_at: oldest,
            newest_valid_at: newest,
            avg_confidence,
        })
    }

    // --- Dump / pagination ---

    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let points = self.scroll_all(&collection, None, true).await?;
        Ok(points
            .iter()
            .map(|p| {
                let emb = Self::extract_vectors(p);
                entity_row_from_payload(&p.payload, emb)
            })
            .collect())
    }

    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        let collection = self.edges_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let points = self.scroll_all(&collection, None, true).await?;
        let mut results: Vec<EdgeRow> = points
            .iter()
            .map(|p| {
                let emb = Self::extract_vectors(p);
                edge_row_from_payload(&p.payload, emb)
            })
            .collect();
        self.enrich_edge_rows(&mut results).await?;
        Ok(results)
    }

    async fn list_entities_by_recency(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EntityRow>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let points = self.scroll_all(&collection, None, true).await?;
        let mut entities: Vec<EntityRow> = points
            .iter()
            .map(|p| {
                let emb = Self::extract_vectors(p);
                entity_row_from_payload(&p.payload, emb)
            })
            .collect();
        entities.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(entities.into_iter().skip(offset).take(limit).collect())
    }

    // --- Provenance ---

    async fn get_provenance(&self, edge_id: i64) -> Result<ProvenanceResponse> {
        // Qdrant doesn't have a supersession table; return empty provenance.
        // Supersession tracking would require a separate collection or external state.
        Ok(ProvenanceResponse {
            edge_id,
            superseded_by: None,
            supersedes: vec![],
        })
    }

    // --- Discovery ---

    async fn find_close_unlinked(
        &self,
        node_id: &str,
        embedding: &[f32],
        threshold: f32,
    ) -> Result<Vec<(EntityRow, f32)>> {
        let entities_col = self.entities_collection();
        let edges_col = self.edges_collection();

        if !self.collection_exists(&entities_col).await? {
            return Ok(vec![]);
        }

        // Find entities linked to node_id
        let mut linked: HashSet<String> = HashSet::new();
        linked.insert(node_id.to_string());
        if self.collection_exists(&edges_col).await? {
            let filter = Filter::must(vec![
                Condition::is_null("invalid_at"),
                Condition::from(Filter::should(vec![
                    Condition::matches("from_id", node_id.to_string()),
                    Condition::matches("to_id", node_id.to_string()),
                ])),
            ]);
            let edge_points = self.scroll_all(&edges_col, Some(filter), false).await?;
            for p in &edge_points {
                linked.insert(payload_str(&p.payload, "from_id"));
                linked.insert(payload_str(&p.payload, "to_id"));
            }
        }

        // Vector search for nearby entities
        let results = self
            .client
            .search_points(
                SearchPointsBuilder::new(&entities_col, embedding.to_vec(), 50)
                    .with_payload(true)
                    .with_vectors(true)
                    .score_threshold(threshold),
            )
            .await?;

        let mut scored: Vec<(EntityRow, f32)> = results
            .result
            .iter()
            .filter_map(|r| {
                let id = payload_str(&r.payload, "id");
                if linked.contains(&id) {
                    return None;
                }
                let emb = r
                    .vectors
                    .as_ref()
                    .and_then(|vs| vs.get_vector())
                    .and_then(|v| {
                        use qdrant_client::qdrant::vector_output::Vector;
                        match v {
                            Vector::Dense(dv) => Some(dv.data),
                            _ => None,
                        }
                    })
                    .unwrap_or_default();
                Some((entity_row_from_payload(&r.payload, emb), r.score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(scored)
    }

    async fn find_placeholder_nodes(&self, _cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(vec![]);
        }

        let filter = Filter::must(vec![Condition::matches("resolved", false)]);
        let points = self.scroll_all(&collection, Some(filter), true).await?;
        Ok(points
            .iter()
            .map(|p| {
                let emb = Self::extract_vectors(p);
                entity_row_from_payload(&p.payload, emb)
            })
            .collect())
    }

    // --- Entity updates ---

    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(());
        }

        let point_id = hash_id(entity_id);
        let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
        payload.insert("name".into(), qdrant_value_str(new_name));

        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(&collection, payload)
                    .points_selector(PointsIdsList::from(vec![PointId::from(point_id)]))
                    .wait(true),
            )
            .await?;
        Ok(())
    }

    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(());
        }

        let point_id = hash_id(entity_id);
        let prop_key = format!("prop_{key}");
        let mut payload: HashMap<String, qdrant_client::qdrant::Value> = HashMap::new();
        payload.insert(prop_key, qdrant_value_str(value));

        self.client
            .set_payload(
                SetPayloadPointsBuilder::new(&collection, payload)
                    .points_selector(PointsIdsList::from(vec![PointId::from(point_id)]))
                    .wait(true),
            )
            .await?;
        Ok(())
    }

    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> {
        let collection = self.entities_collection();
        if !self.collection_exists(&collection).await? {
            return Ok(None);
        }

        let prop_key = format!("prop_{key}");
        let filter = Filter::must(vec![Condition::matches(
            prop_key.as_str(),
            value.to_string(),
        )]);
        let points = self.scroll_all(&collection, Some(filter), true).await?;

        Ok(points.first().map(|p| {
            let emb = Self::extract_vectors(p);
            entity_row_from_payload(&p.payload, emb)
        }))
    }
}
