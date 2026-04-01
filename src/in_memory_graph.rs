use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;

use crate::credibility::SourceCredibility;
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

fn tier_string(tier: &MemoryTier) -> String {
    match tier {
        MemoryTier::Working => "working".to_string(),
        MemoryTier::LongTerm => "long_term".to_string(),
    }
}

fn parse_tier(s: &str) -> MemoryTier {
    if s == "working" {
        MemoryTier::Working
    } else {
        MemoryTier::LongTerm
    }
}

struct StoredEdge {
    edge_id: i64,
    from_id: String,
    to_id: String,
    fact: String,
    relation_type: String,
    confidence: f32,
    salience: i64,
    valid_at: DateTime<Utc>,
    invalid_at: Option<DateTime<Utc>>,
    embedding: Vec<f32>,
    source_agents: Vec<String>,
    memory_tier: MemoryTier,
    created_at: DateTime<Utc>,
    decayed_confidence: f32,
    expires_at: Option<DateTime<Utc>>,
}

impl StoredEdge {
    fn is_active(&self) -> bool {
        self.invalid_at.is_none()
    }

    fn is_active_at(&self, at: DateTime<Utc>) -> bool {
        self.valid_at <= at && self.invalid_at.map_or(true, |inv| inv > at)
    }

    fn to_row(&self, entities: &HashMap<String, EntityRow>) -> EdgeRow {
        let from_name = entities.get(&self.from_id).map_or("", |e| &e.name).to_string();
        let to_name = entities.get(&self.to_id).map_or("", |e| &e.name).to_string();
        EdgeRow {
            edge_id: self.edge_id,
            subject_id: self.from_id.clone(),
            subject_name: from_name,
            fact: self.fact.clone(),
            relation_type: self.relation_type.clone(),
            confidence: self.confidence,
            salience: self.salience,
            valid_at: self.valid_at.to_rfc3339(),
            invalid_at: self.invalid_at.map(|t| t.to_rfc3339()),
            object_id: self.to_id.clone(),
            object_name: to_name,
            embedding: self.embedding.clone(),
            decayed_confidence: self.decayed_confidence,
            source_agents: self.source_agents.join(","),
            memory_tier: tier_string(&self.memory_tier),
            expires_at: self.expires_at.map(|t| t.to_rfc3339()),
        }
    }
}

pub struct InMemoryGraph {
    name: String,
    entities: RwLock<HashMap<String, EntityRow>>,
    edges: RwLock<Vec<StoredEdge>>,
    next_edge_id: AtomicI64,
    supersessions: RwLock<Vec<SupersessionRecord>>,
    source_credibility: RwLock<Vec<SourceCredibility>>,
    properties: RwLock<HashMap<(String, String), String>>,
}

impl InMemoryGraph {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            entities: RwLock::new(HashMap::new()),
            edges: RwLock::new(Vec::new()),
            next_edge_id: AtomicI64::new(1),
            supersessions: RwLock::new(Vec::new()),
            source_credibility: RwLock::new(Vec::new()),
            properties: RwLock::new(HashMap::new()),
        }
    }

    async fn walk_one_hop_inner(
        &self,
        entity_ids: &[String],
        limit: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;
        let id_set: std::collections::HashSet<&str> =
            entity_ids.iter().map(|s| s.as_str()).collect();
        Ok(edges
            .iter()
            .filter(|e| {
                let active = match at {
                    Some(t) => e.is_active_at(t),
                    None => e.is_active(),
                };
                active
                    && (id_set.contains(e.from_id.as_str())
                        || id_set.contains(e.to_id.as_str()))
            })
            .take(limit)
            .map(|e| e.to_row(&entities))
            .collect())
    }
}

#[async_trait]
impl GraphBackend for InMemoryGraph {
    fn graph_name(&self) -> &str { &self.name }
    async fn ping(&self) -> Result<()> {
        Ok(())
    }

    async fn setup_schema(&self) -> Result<()> {
        Ok(())
    }

    async fn drop_and_reinitialise(&self) -> Result<()> {
        self.entities.write().await.clear();
        self.edges.write().await.clear();
        self.supersessions.write().await.clear();
        self.source_credibility.write().await.clear();
        self.properties.write().await.clear();
        Ok(())
    }

    // --- Entity search ---

    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>> {
        let lower = query_str.to_lowercase();
        let entities = self.entities.read().await;
        Ok(entities
            .values()
            .filter(|e| e.name.to_lowercase().contains(&lower))
            .cloned()
            .collect())
    }

    async fn vector_search_entities(
        &self,
        embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(EntityRow, f32)>> {
        let entities = self.entities.read().await;
        let mut scored: Vec<(EntityRow, f32)> = entities
            .values()
            .filter(|e| !e.embedding.is_empty())
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (e.clone(), score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }

    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        Ok(self.entities.read().await.get(entity_id).cloned())
    }

    // --- Edge search ---

    async fn fulltext_search_edges(&self, query_str: &str, at: Option<DateTime<Utc>>) -> Result<Vec<EdgeRow>> {
        let lower = query_str.to_lowercase();
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;
        Ok(edges
            .iter()
            .filter(|e| {
                let active = match at {
                    Some(t) => e.is_active_at(t),
                    None => e.is_active(),
                };
                active && e.fact.to_lowercase().contains(&lower)
            })
            .map(|e| e.to_row(&entities))
            .collect())
    }

    async fn vector_search_edges_scored(
        &self,
        embedding: &[f32],
        k: usize,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<(EdgeRow, f32)>> {
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;
        let mut scored: Vec<(EdgeRow, f32)> = edges
            .iter()
            .filter(|e| {
                let active = match at {
                    Some(t) => e.is_active_at(t),
                    None => e.is_active(),
                };
                active && !e.embedding.is_empty()
            })
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (e.to_row(&entities), score)
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
        let mut results = Vec::new();
        let mut frontier: Vec<String> = seed_entity_ids.to_vec();
        let mut visited_edges: std::collections::HashSet<i64> = std::collections::HashSet::new();

        for hop in 1..=max_hops {
            let hop_edges = self.walk_one_hop_inner(&frontier, limit_per_hop, at).await?;
            let mut next_frontier = Vec::new();
            for edge in hop_edges {
                if visited_edges.insert(edge.edge_id) {
                    // Discover new entity IDs from this edge
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

    async fn entity_timeline(&self, entity_name: &str) -> Result<Vec<EdgeRow>> {
        let lower = entity_name.to_lowercase();
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;
        let mut matching: Vec<_> = edges
            .iter()
            .filter(|e| {
                let from_name = entities.get(&e.from_id).map_or("", |ent| &ent.name);
                let to_name = entities.get(&e.to_id).map_or("", |ent| &ent.name);
                from_name.to_lowercase().contains(&lower)
                    || to_name.to_lowercase().contains(&lower)
            })
            .collect();
        matching.sort_by_key(|e| e.valid_at);
        Ok(matching.iter().map(|e| e.to_row(&entities)).collect())
    }

    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>> {
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;
        Ok(edges
            .iter()
            .filter(|e| e.is_active() && (e.from_id == node_id || e.to_id == node_id))
            .map(|e| e.to_row(&entities))
            .collect())
    }

    // --- Mutation ---

    async fn upsert_entity(&self, entity: &Entity) -> Result<()> {
        let row = EntityRow {
            id: entity.id.clone(),
            name: entity.name.clone(),
            entity_type: entity.entity_type.clone(),
            resolved: entity.resolved,
            hint: entity.hint.clone(),
            content: entity.content.clone(),
            created_at: entity.created_at.to_rfc3339(),
            embedding: entity.embedding.clone(),
        };
        self.entities.write().await.insert(entity.id.clone(), row);
        Ok(())
    }

    async fn create_edge(
        &self,
        from_id: &str,
        to_id: &str,
        rel: &Relation,
    ) -> Result<i64> {
        let edge_id = self.next_edge_id.fetch_add(1, Ordering::Relaxed);
        let edge = StoredEdge {
            edge_id,
            from_id: from_id.to_string(),
            to_id: to_id.to_string(),
            fact: rel.fact.clone(),
            relation_type: rel.relation_type.clone(),
            confidence: rel.confidence,
            salience: rel.salience,
            valid_at: rel.valid_at,
            invalid_at: rel.invalid_at,
            embedding: rel.embedding.clone(),
            source_agents: rel.source_agents.clone(),
            memory_tier: rel.memory_tier.clone(),
            created_at: rel.created_at,
            decayed_confidence: rel.confidence,
            expires_at: rel.expires_at,
        };
        self.edges.write().await.push(edge);
        Ok(edge_id)
    }

    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()> {
        let mut edges = self.edges.write().await;
        if let Some(e) = edges.iter_mut().find(|e| e.edge_id == edge_id) {
            e.invalid_at = Some(at);
        }
        Ok(())
    }

    async fn increment_salience(&self, edge_ids: &[i64]) -> Result<()> {
        let mut edges = self.edges.write().await;
        for e in edges.iter_mut() {
            if edge_ids.contains(&e.edge_id) {
                e.salience += 1;
            }
        }
        Ok(())
    }

    async fn delete_entity(&self, entity_id: &str) -> Result<usize> {
        let now = Utc::now();
        let mut edges = self.edges.write().await;
        let mut count = 0;
        for e in edges.iter_mut() {
            if e.is_active() && (e.from_id == entity_id || e.to_id == entity_id) {
                e.invalid_at = Some(now);
                count += 1;
            }
        }
        drop(edges);
        self.entities.write().await.remove(entity_id);
        Ok(count)
    }

    async fn merge_placeholder(
        &self,
        placeholder_id: &str,
        resolved_id: &str,
    ) -> Result<()> {
        let mut edges = self.edges.write().await;
        for e in edges.iter_mut() {
            if e.from_id == placeholder_id {
                e.from_id = resolved_id.to_string();
            }
            if e.to_id == placeholder_id {
                e.to_id = resolved_id.to_string();
            }
        }
        self.entities.write().await.remove(placeholder_id);
        Ok(())
    }

    // --- Memory tier management ---

    async fn promote_working_memory(&self) -> Result<usize> {
        let mut edges = self.edges.write().await;
        let threshold = Utc::now() - Duration::hours(1);
        let mut count = 0;
        for e in edges.iter_mut() {
            if matches!(e.memory_tier, MemoryTier::Working)
                && e.is_active()
                && e.salience >= 3
                && e.created_at < threshold
            {
                e.memory_tier = MemoryTier::LongTerm;
                count += 1;
            }
        }
        Ok(count)
    }

    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize> {
        let mut edges = self.edges.write().await;
        let mut count = 0;
        for e in edges.iter_mut() {
            if e.is_active() {
                if let Some(exp) = e.expires_at {
                    if exp <= now {
                        e.invalid_at = Some(now);
                        count += 1;
                    }
                }
            }
        }
        Ok(count)
    }

    async fn memory_tier_stats(&self) -> Result<crate::models::MemoryTierStats> {
        let edges = self.edges.read().await;
        let working_count = edges
            .iter()
            .filter(|e| e.is_active() && matches!(e.memory_tier, MemoryTier::Working))
            .count();
        let long_term_count = edges
            .iter()
            .filter(|e| e.is_active() && matches!(e.memory_tier, MemoryTier::LongTerm))
            .count();
        Ok(crate::models::MemoryTierStats { working_count, long_term_count })
    }

    async fn decay_stale_edges(
        &self,
        stale_before: DateTime<Utc>,
        _now: DateTime<Utc>,
    ) -> Result<usize> {
        let mut edges = self.edges.write().await;
        let mut count = 0;
        for e in edges.iter_mut() {
            if e.is_active() && e.valid_at < stale_before {
                e.decayed_confidence *= 0.95;
                count += 1;
            }
        }
        Ok(count)
    }

    // --- Facts / reflection ---

    async fn entity_facts(&self, entity_name: &str) -> Result<Vec<EdgeRow>> {
        let lower = entity_name.to_lowercase();
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;

        // Find entity IDs matching the name
        let matching_ids: Vec<&str> = entities
            .values()
            .filter(|e| e.name.to_lowercase() == lower)
            .map(|e| e.id.as_str())
            .collect();

        Ok(edges
            .iter()
            .filter(|e| {
                e.is_active()
                    && (matching_ids.contains(&e.from_id.as_str())
                        || matching_ids.contains(&e.to_id.as_str()))
            })
            .map(|e| e.to_row(&entities))
            .collect())
    }

    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        let edges = self.edges.read().await;
        Ok(edges
            .iter()
            .filter(|e| {
                e.is_active() && (e.from_id == entity_id || e.to_id == entity_id)
            })
            .map(|e| e.fact.clone())
            .collect())
    }

    async fn graph_stats(&self) -> Result<crate::models::GraphStats> {
        let entities = self.entities.read().await;
        let edges = self.edges.read().await;
        let active: Vec<&StoredEdge> = edges.iter().filter(|e| e.is_active()).collect();
        let edge_count = active.len();
        let avg_confidence = if edge_count > 0 {
            active.iter().map(|e| e.confidence).sum::<f32>() / edge_count as f32
        } else {
            0.0
        };
        let oldest_valid_at = active.iter().map(|e| e.valid_at).min().map(|t| t.to_rfc3339());
        let newest_valid_at = active.iter().map(|e| e.valid_at).max().map(|t| t.to_rfc3339());
        Ok(crate::models::GraphStats {
            entity_count: entities.len(),
            edge_count,
            oldest_valid_at,
            newest_valid_at,
            avg_confidence,
        })
    }

    async fn all_relation_types(&self) -> Result<Vec<String>> {
        let edges = self.edges.read().await;
        let mut types: Vec<String> = edges
            .iter()
            .filter(|e| e.is_active())
            .map(|e| e.relation_type.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        types.sort();
        Ok(types)
    }

    async fn under_documented_entities(
        &self,
        threshold: usize,
    ) -> Result<Vec<crate::models::UnderDocumentedEntity>> {
        let entities = self.entities.read().await;
        let edges = self.edges.read().await;
        let mut result = Vec::new();
        for entity in entities.values() {
            let count = edges
                .iter()
                .filter(|e| {
                    e.is_active() && (e.from_id == entity.id || e.to_id == entity.id)
                })
                .count();
            if count < threshold {
                result.push(crate::models::UnderDocumentedEntity {
                    id: entity.id.clone(),
                    name: entity.name.clone(),
                    edge_count: count,
                });
            }
        }
        Ok(result)
    }

    async fn entity_type_counts(&self) -> Result<HashMap<String, usize>> {
        let entities = self.entities.read().await;
        let mut counts = HashMap::new();
        for entity in entities.values() {
            *counts.entry(entity.entity_type.clone()).or_insert(0) += 1;
        }
        Ok(counts)
    }

    // --- Dump / pagination ---

    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>> {
        Ok(self.entities.read().await.values().cloned().collect())
    }

    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>> {
        let edges = self.edges.read().await;
        let entities = self.entities.read().await;
        Ok(edges.iter().map(|e| e.to_row(&entities)).collect())
    }

    async fn list_entities_by_recency(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<EntityRow>> {
        let entities = self.entities.read().await;
        let mut sorted: Vec<EntityRow> = entities.values().cloned().collect();
        sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(sorted.into_iter().skip(offset).take(limit).collect())
    }

    // --- Supersession / provenance ---

    async fn get_provenance(&self, edge_id: i64) -> Result<ProvenanceResponse> {
        let sups = self.supersessions.read().await;
        let superseded_by = sups
            .iter()
            .find(|s| s.old_edge_id == edge_id)
            .cloned();
        let supersedes: Vec<SupersessionRecord> = sups
            .iter()
            .filter(|s| s.new_edge_id == edge_id)
            .cloned()
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
        let entities = self.entities.read().await;
        let edges = self.edges.read().await;

        // Find entities connected to node_id
        let linked: std::collections::HashSet<String> = edges
            .iter()
            .filter(|e| e.is_active() && (e.from_id == node_id || e.to_id == node_id))
            .flat_map(|e| {
                vec![e.from_id.clone(), e.to_id.clone()]
            })
            .collect();

        let mut results: Vec<(EntityRow, f32)> = entities
            .values()
            .filter(|e| e.id != node_id && !linked.contains(&e.id) && !e.embedding.is_empty())
            .map(|e| {
                let score = cosine_similarity(embedding, &e.embedding);
                (e.clone(), score)
            })
            .filter(|(_, score)| *score >= threshold)
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }

    async fn find_placeholder_nodes(
        &self,
        _cutoff: DateTime<Utc>,
    ) -> Result<Vec<EntityRow>> {
        let entities = self.entities.read().await;
        Ok(entities
            .values()
            .filter(|e| !e.resolved)
            .cloned()
            .collect())
    }

    async fn find_two_hop_unlinked_pairs(
        &self,
        limit: usize,
    ) -> Result<Vec<(EntityRow, EntityRow)>> {
        let entities = self.entities.read().await;
        let edges = self.edges.read().await;
        let mut pairs = Vec::new();

        // Build adjacency: entity_id -> set of connected entity_ids
        let mut adj: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
        for e in edges.iter().filter(|e| e.is_active()) {
            adj.entry(&e.from_id).or_default().insert(&e.to_id);
            adj.entry(&e.to_id).or_default().insert(&e.from_id);
        }

        let entity_ids: Vec<&str> = entities.keys().map(|s| s.as_str()).collect();
        'outer: for (i, &a_id) in entity_ids.iter().enumerate() {
            let a_neighbors = adj.get(a_id).cloned().unwrap_or_default();
            for &b_id in entity_ids.iter().skip(i + 1) {
                if a_neighbors.contains(b_id) {
                    continue; // directly linked
                }
                // Check if two-hop connected
                let b_neighbors = adj.get(b_id).cloned().unwrap_or_default();
                let shared = a_neighbors.intersection(&b_neighbors).count();
                if shared > 0 {
                    if let (Some(a), Some(b)) = (entities.get(a_id), entities.get(b_id)) {
                        pairs.push((a.clone(), b.clone()));
                        if pairs.len() >= limit {
                            break 'outer;
                        }
                    }
                }
            }
        }
        Ok(pairs)
    }

    // --- Archive ---

    async fn archive_low_confidence_edges(
        &self,
        threshold: f32,
        dry_run: bool,
    ) -> Result<Vec<EdgeRow>> {
        let mut edges = self.edges.write().await;
        let entities = self.entities.read().await;
        let now = Utc::now();
        let mut archived = Vec::new();
        for e in edges.iter_mut() {
            if e.is_active() && e.confidence < threshold {
                archived.push(e.to_row(&entities));
                if !dry_run {
                    e.invalid_at = Some(now);
                }
            }
        }
        Ok(archived)
    }

    // --- Entity updates ---

    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        let mut entities = self.entities.write().await;
        if let Some(e) = entities.get_mut(entity_id) {
            e.name = new_name.to_string();
        }
        Ok(())
    }

    async fn set_entity_property(
        &self,
        entity_id: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        self.properties.write().await.insert(
            (entity_id.to_string(), key.to_string()),
            value.to_string(),
        );
        Ok(())
    }

    async fn find_entity_by_property(
        &self,
        key: &str,
        value: &str,
    ) -> Result<Option<EntityRow>> {
        let props = self.properties.read().await;
        let entities = self.entities.read().await;
        for ((eid, k), v) in props.iter() {
            if k == key && v == value {
                if let Some(entity) = entities.get(eid) {
                    return Ok(Some(entity.clone()));
                }
            }
        }
        Ok(None)
    }

    // --- Clustering ---

    async fn find_entity_clusters(&self, min_size: usize) -> Result<Vec<Vec<String>>> {
        // Simple connected components via union-find
        let entities = self.entities.read().await;
        let edges = self.edges.read().await;

        let mut parent: HashMap<String, String> = HashMap::new();
        for id in entities.keys() {
            parent.insert(id.clone(), id.clone());
        }

        fn find(parent: &mut HashMap<String, String>, x: &str) -> String {
            let p = parent.get(x).cloned().unwrap_or_else(|| x.to_string());
            if p == x {
                return p;
            }
            let root = find(parent, &p);
            parent.insert(x.to_string(), root.clone());
            root
        }

        for e in edges.iter().filter(|e| e.is_active()) {
            let root_a = find(&mut parent, &e.from_id);
            let root_b = find(&mut parent, &e.to_id);
            if root_a != root_b {
                parent.insert(root_a, root_b);
            }
        }

        let mut clusters: HashMap<String, Vec<String>> = HashMap::new();
        for id in entities.keys() {
            let root = find(&mut parent, id);
            clusters.entry(root).or_default().push(id.clone());
        }

        Ok(clusters
            .into_values()
            .filter(|c| c.len() >= min_size)
            .collect())
    }

}

impl InMemoryGraph {
    pub async fn compound_edge_confidence(
        &self,
        edge_id: i64,
        new_agent: &str,
        new_confidence: f32,
    ) -> Result<f32> {
        let mut edges = self.edges.write().await;
        if let Some(e) = edges.iter_mut().find(|e| e.edge_id == edge_id) {
            if !e.source_agents.contains(&new_agent.to_string()) {
                e.source_agents.push(new_agent.to_string());
            }
            let combined = 1.0 - (1.0 - e.confidence) * (1.0 - new_confidence);
            e.confidence = combined;
            e.decayed_confidence = combined;
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
        self.supersessions.write().await.push(SupersessionRecord {
            old_edge_id,
            new_edge_id,
            superseded_at,
            old_fact: old_fact.to_string(),
            new_fact: new_fact.to_string(),
        });
        Ok(())
    }

    pub async fn get_supersession_chain(&self, edge_id: i64) -> Result<Vec<SupersessionRecord>> {
        let sups = self.supersessions.read().await;
        Ok(sups
            .iter()
            .filter(|s| s.old_edge_id == edge_id || s.new_edge_id == edge_id)
            .cloned()
            .collect())
    }

    pub async fn save_source_credibility(&self, cred: &SourceCredibility) -> Result<()> {
        let mut store = self.source_credibility.write().await;
        if let Some(existing) = store.iter_mut().find(|c| c.agent_id == cred.agent_id) {
            *existing = cred.clone();
        } else {
            store.push(cred.clone());
        }
        Ok(())
    }

    pub async fn load_all_source_credibility(&self) -> Result<Vec<SourceCredibility>> {
        Ok(self.source_credibility.read().await.clone())
    }
}
