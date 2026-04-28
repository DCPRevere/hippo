use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;

use crate::credibility::SourceCredibility;
use crate::graph_backend::GraphBackend;
use crate::math::{compound_confidence, cosine_similarity};
use crate::models::{
    EdgeRow, Entity, EntityRow, MemoryTier, ProvenanceResponse, Relation, SupersessionRecord,
};

fn tier_string(tier: &MemoryTier) -> String {
    match tier {
        MemoryTier::Working => "working".to_string(),
        MemoryTier::LongTerm => "long_term".to_string(),
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
        self.valid_at <= at && self.invalid_at.is_none_or(|inv| inv > at)
    }

    fn to_row(&self, entities: &HashMap<String, EntityRow>) -> EdgeRow {
        let from_name = entities
            .get(&self.from_id)
            .map_or("", |e| &e.name)
            .to_string();
        let to_name = entities
            .get(&self.to_id)
            .map_or("", |e| &e.name)
            .to_string();
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
    last_visited: RwLock<HashMap<String, DateTime<Utc>>>,
    retraction_reasons: RwLock<HashMap<i64, String>>,
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
            last_visited: RwLock::new(HashMap::new()),
            retraction_reasons: RwLock::new(HashMap::new()),
        }
    }

    // ---- Dreamer support: salience, supersession, last-visited ----
    //
    // These methods support the architecture described in docs/DREAMS.md.
    // They are implemented here as inherent methods on InMemoryGraph; the
    // GraphBackend trait will gain them once the SQLite/Postgres/Qdrant
    // backends grow parity.

    /// Return entities whose last_visited is older than `cutoff` (or has never
    /// been visited). The Dreamer's work query.
    pub async fn entities_unvisited_since(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<EntityRow>> {
        let entities = self.entities.read().await;
        let visited = self.last_visited.read().await;
        let mut out = Vec::new();
        for (id, row) in entities.iter() {
            let eligible = match visited.get(id) {
                Some(ts) => *ts < cutoff,
                None => true,
            };
            if eligible {
                out.push(row.clone());
            }
        }
        Ok(out)
    }

    /// Return the recorded retraction reason for an edge, if any.
    pub async fn retraction_reason(&self, edge_id: i64) -> Result<Option<String>> {
        Ok(self.retraction_reasons.read().await.get(&edge_id).cloned())
    }

    /// Convenience: retract the old edge and observe a replacement. Returns
    /// the new edge id. Atomic-ish (the two writes are sequential, but the
    /// in-memory backend serialises them deterministically).
    pub async fn correct_edge(
        &self,
        old_edge_id: i64,
        from_id: &str,
        to_id: &str,
        new_rel: &Relation,
        reason: Option<&str>,
    ) -> Result<i64> {
        self.retract_edge(old_edge_id, reason).await?;
        self.create_edge(from_id, to_id, new_rel).await
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
                active && (id_set.contains(e.from_id.as_str()) || id_set.contains(e.to_id.as_str()))
            })
            .take(limit)
            .map(|e| e.to_row(&entities))
            .collect())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl GraphBackend for InMemoryGraph {
    fn graph_name(&self) -> &str {
        &self.name
    }
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
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(k);
        Ok(scored)
    }

    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>> {
        Ok(self.entities.read().await.get(entity_id).cloned())
    }

    // --- Edge search ---

    async fn fulltext_search_edges(
        &self,
        query_str: &str,
        at: Option<DateTime<Utc>>,
    ) -> Result<Vec<EdgeRow>> {
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
        let supersessions = self.supersessions.read().await;
        let credibility = self.source_credibility.read().await;

        // Build a set of edge_ids that have been superseded by another fact.
        // Append-only dreaming writes a `supersedes` relationship instead of
        // mutating `invalid_at`; retrieval is responsible for filtering them
        // from active search results.
        let superseded: std::collections::HashSet<i64> =
            supersessions.iter().map(|s| s.old_edge_id).collect();

        let cred_lookup: HashMap<&str, f32> = credibility
            .iter()
            .map(|c| (c.agent_id.as_str(), c.credibility))
            .collect();

        let mut scored: Vec<(EdgeRow, f32)> = edges
            .iter()
            .filter(|e| {
                let active = match at {
                    Some(t) => e.is_active_at(t),
                    None => e.is_active(),
                };
                active && !e.embedding.is_empty() && !superseded.contains(&e.edge_id)
            })
            .map(|e| {
                let similarity = cosine_similarity(embedding, &e.embedding);
                // Salience boost: log1p damps so a few uses don't dominate
                // similarity, but compounding bumps are visible. Weight is
                // small (~0.01 per natural-log unit) so similarity remains
                // primary signal.
                let salience_boost = (e.salience as f32).max(0.0).ln_1p() * 0.01;
                // Credibility multiplier: defaults to 0.8 for unknown sources
                // (matches CredibilityRegistry default). Average across all
                // listed source agents on the edge.
                let cred = if e.source_agents.is_empty() {
                    0.8
                } else {
                    let total: f32 = e
                        .source_agents
                        .iter()
                        .map(|s| cred_lookup.get(s.as_str()).copied().unwrap_or(0.8))
                        .sum();
                    total / e.source_agents.len() as f32
                };
                let score = (similarity + salience_boost) * cred;
                (e.to_row(&entities), score)
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
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
            let hop_edges = self
                .walk_one_hop_inner(&frontier, limit_per_hop, at)
                .await?;
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

    async fn create_edge(&self, from_id: &str, to_id: &str, rel: &Relation) -> Result<i64> {
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

    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()> {
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
        Ok(crate::models::MemoryTierStats {
            working_count,
            long_term_count,
        })
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

    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>> {
        let edges = self.edges.read().await;
        Ok(edges
            .iter()
            .filter(|e| e.is_active() && (e.from_id == entity_id || e.to_id == entity_id))
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
        let oldest_valid_at = active
            .iter()
            .map(|e| e.valid_at)
            .min()
            .map(|t| t.to_rfc3339());
        let newest_valid_at = active
            .iter()
            .map(|e| e.valid_at)
            .max()
            .map(|t| t.to_rfc3339());
        Ok(crate::models::GraphStats {
            entity_count: entities.len(),
            edge_count,
            oldest_valid_at,
            newest_valid_at,
            avg_confidence,
        })
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
        let superseded_by = sups.iter().find(|s| s.old_edge_id == edge_id).cloned();
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
            .flat_map(|e| vec![e.from_id.clone(), e.to_id.clone()])
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

        results.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(results)
    }

    async fn find_placeholder_nodes(&self, _cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>> {
        let entities = self.entities.read().await;
        Ok(entities.values().filter(|e| !e.resolved).cloned().collect())
    }

    // --- Archive ---

    // --- Entity updates ---

    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()> {
        let mut entities = self.entities.write().await;
        if let Some(e) = entities.get_mut(entity_id) {
            e.name = new_name.to_string();
        }
        Ok(())
    }

    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()> {
        self.properties
            .write()
            .await
            .insert((entity_id.to_string(), key.to_string()), value.to_string());
        Ok(())
    }

    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>> {
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

    // --- Dreamer support (trait overrides) ---

    async fn bump_salience(&self, edge_ids: &[i64]) -> Result<()> {
        let mut edges = self.edges.write().await;
        for edge in edges.iter_mut() {
            if edge_ids.contains(&edge.edge_id) {
                edge.salience = edge.salience.saturating_add(1);
            }
        }
        Ok(())
    }

    async fn supersede_edge(&self, old_edge_id: i64, new_edge_id: i64) -> Result<()> {
        let mut sups = self.supersessions.write().await;
        if sups
            .iter()
            .any(|s| s.old_edge_id == old_edge_id && s.new_edge_id == new_edge_id)
        {
            return Ok(());
        }
        let edges = self.edges.read().await;
        let old_fact = edges
            .iter()
            .find(|e| e.edge_id == old_edge_id)
            .map(|e| e.fact.clone())
            .unwrap_or_default();
        let new_fact = edges
            .iter()
            .find(|e| e.edge_id == new_edge_id)
            .map(|e| e.fact.clone())
            .unwrap_or_default();
        sups.push(SupersessionRecord {
            old_edge_id,
            new_edge_id,
            superseded_at: Utc::now(),
            old_fact,
            new_fact,
        });
        Ok(())
    }

    async fn retract_edge(&self, edge_id: i64, reason: Option<&str>) -> Result<()> {
        let now = Utc::now();
        {
            let mut edges = self.edges.write().await;
            for edge in edges.iter_mut() {
                if edge.edge_id == edge_id && edge.invalid_at.is_none() {
                    edge.invalid_at = Some(now);
                }
            }
        }
        if let Some(r) = reason {
            self.retraction_reasons
                .write()
                .await
                .insert(edge_id, r.to_string());
        }
        Ok(())
    }

    async fn mark_visited(&self, entity_id: &str, at: DateTime<Utc>) -> Result<()> {
        self.last_visited
            .write()
            .await
            .insert(entity_id.to_string(), at);
        Ok(())
    }

    async fn last_visited(&self, entity_id: &str) -> Result<Option<DateTime<Utc>>> {
        Ok(self.last_visited.read().await.get(entity_id).copied())
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
            let combined = compound_confidence(e.confidence, new_confidence);
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn entity(id: &str, name: &str, kind: &str) -> Entity {
        Entity {
            id: id.into(),
            name: name.into(),
            entity_type: kind.into(),
            resolved: true,
            hint: None,
            content: None,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            embedding: vec![0.1, 0.2, 0.3],
        }
    }

    fn relation(fact: &str, rel: &str, valid_at: DateTime<Utc>) -> Relation {
        Relation {
            fact: fact.into(),
            relation_type: rel.into(),
            embedding: vec![0.1, 0.2, 0.3],
            source_agents: vec!["test".into()],
            valid_at,
            invalid_at: None,
            confidence: 0.9,
            salience: 1,
            created_at: valid_at,
            memory_tier: MemoryTier::Working,
            expires_at: None,
        }
    }

    async fn populated_graph() -> InMemoryGraph {
        let g = InMemoryGraph::new("test");
        g.upsert_entity(&entity("a", "Alice", "person")).await.unwrap();
        g.upsert_entity(&entity("b", "Bob", "person")).await.unwrap();
        g.upsert_entity(&entity("c", "Acme", "org")).await.unwrap();
        g
    }

    // ---- Identity & lifecycle ----

    #[tokio::test]
    async fn graph_name_returns_constructor_value() {
        let g = InMemoryGraph::new("hippo-test");
        assert_eq!(g.graph_name(), "hippo-test");
    }

    #[tokio::test]
    async fn ping_succeeds_on_fresh_graph() {
        let g = InMemoryGraph::new("x");
        g.ping().await.unwrap();
    }

    #[tokio::test]
    async fn drop_and_reinitialise_clears_all_state() {
        let g = populated_graph().await;
        let now = Utc::now();
        g.create_edge(
            "a",
            "b",
            &relation("Alice knows Bob", "KNOWS", now),
        )
        .await
        .unwrap();
        g.drop_and_reinitialise().await.unwrap();
        assert!(g.dump_all_entities().await.unwrap().is_empty());
        assert!(g.dump_all_edges().await.unwrap().is_empty());
    }

    // ---- Entity CRUD ----

    #[tokio::test]
    async fn upsert_then_get_entity_round_trips_fields() {
        let g = InMemoryGraph::new("x");
        g.upsert_entity(&entity("a", "Alice", "person"))
            .await
            .unwrap();
        let row = g.get_entity_by_id("a").await.unwrap().unwrap();
        assert_eq!(row.name, "Alice");
        assert_eq!(row.entity_type, "person");
    }

    #[tokio::test]
    async fn get_entity_by_id_returns_none_for_missing() {
        let g = InMemoryGraph::new("x");
        assert!(g.get_entity_by_id("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn upsert_overwrites_existing_entity() {
        let g = InMemoryGraph::new("x");
        g.upsert_entity(&entity("a", "Alice", "person"))
            .await
            .unwrap();
        let mut updated = entity("a", "Alice2", "person");
        updated.hint = Some("note".into());
        g.upsert_entity(&updated).await.unwrap();
        let row = g.get_entity_by_id("a").await.unwrap().unwrap();
        assert_eq!(row.name, "Alice2");
        assert_eq!(row.hint.as_deref(), Some("note"));
    }

    #[tokio::test]
    async fn rename_entity_updates_name_only() {
        let g = populated_graph().await;
        g.rename_entity("a", "Alicia").await.unwrap();
        let row = g.get_entity_by_id("a").await.unwrap().unwrap();
        assert_eq!(row.name, "Alicia");
        assert_eq!(row.entity_type, "person");
    }

    #[tokio::test]
    async fn rename_missing_entity_is_silent_noop() {
        let g = InMemoryGraph::new("x");
        // Documents current behaviour: rename of a missing id does not error.
        g.rename_entity("ghost", "Whatever").await.unwrap();
    }

    // ---- Edge creation & traversal ----

    #[tokio::test]
    async fn create_edge_assigns_unique_increasing_ids() {
        let g = populated_graph().await;
        let now = Utc::now();
        let id1 = g
            .create_edge("a", "b", &relation("f1", "KNOWS", now))
            .await
            .unwrap();
        let id2 = g
            .create_edge("a", "c", &relation("f2", "WORKS_AT", now))
            .await
            .unwrap();
        assert_ne!(id1, id2);
        assert!(id2 > id1);
    }

    #[tokio::test]
    async fn fulltext_search_edges_matches_fact_substring_case_insensitive() {
        let g = populated_graph().await;
        g.create_edge(
            "a",
            "c",
            &relation("Alice works at Acme", "WORKS_AT", Utc::now()),
        )
        .await
        .unwrap();
        let hits = g.fulltext_search_edges("ACME", None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subject_name, "Alice");
        assert_eq!(hits[0].object_name, "Acme");
    }

    #[tokio::test]
    async fn fulltext_search_excludes_invalidated_edges() {
        let g = populated_graph().await;
        let id = g
            .create_edge("a", "c", &relation("Alice works at Acme", "WORKS_AT", Utc::now()))
            .await
            .unwrap();
        g.invalidate_edge(id, Utc::now()).await.unwrap();
        assert!(g.fulltext_search_edges("acme", None).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn walk_n_hops_finds_two_hop_path() {
        let g = populated_graph().await;
        let now = Utc::now();
        g.create_edge("a", "b", &relation("Alice knows Bob", "KNOWS", now))
            .await
            .unwrap();
        g.create_edge("b", "c", &relation("Bob works at Acme", "WORKS_AT", now))
            .await
            .unwrap();

        let hops = g
            .walk_n_hops(&["a".to_string()], 2, 100, None)
            .await
            .unwrap();
        let depths: std::collections::HashSet<usize> = hops.iter().map(|(_, h)| *h).collect();
        assert!(depths.contains(&1));
        assert!(depths.contains(&2));
        assert_eq!(hops.len(), 2);
    }

    #[tokio::test]
    async fn walk_n_hops_respects_max_hops_limit() {
        let g = populated_graph().await;
        let now = Utc::now();
        g.create_edge("a", "b", &relation("ab", "KNOWS", now)).await.unwrap();
        g.create_edge("b", "c", &relation("bc", "KNOWS", now)).await.unwrap();
        let hops = g
            .walk_n_hops(&["a".to_string()], 1, 100, None)
            .await
            .unwrap();
        assert_eq!(hops.len(), 1);
    }

    // ---- Temporal filtering ----

    #[tokio::test]
    async fn fulltext_search_at_t_excludes_future_edges() {
        let g = populated_graph().await;
        let past = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let future = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        g.create_edge("a", "b", &relation("Alice knows Bob", "KNOWS", future))
            .await
            .unwrap();
        // At a past time, the edge isn't yet valid.
        assert!(g
            .fulltext_search_edges("alice", Some(past))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn fulltext_search_at_t_excludes_invalidated_edges_by_then() {
        let g = populated_graph().await;
        let t0 = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        let id = g
            .create_edge("a", "b", &relation("Alice knows Bob", "KNOWS", t0))
            .await
            .unwrap();
        g.invalidate_edge(id, t1).await.unwrap();
        // At t2 the edge is invalidated.
        assert!(g
            .fulltext_search_edges("alice", Some(t2))
            .await
            .unwrap()
            .is_empty());
        // Querying without `at` falls back to "currently active" — also empty.
        assert!(g.fulltext_search_edges("alice", None).await.unwrap().is_empty());
    }

    // ---- Vector search ----

    #[tokio::test]
    async fn vector_search_entities_returns_top_k_sorted() {
        let g = InMemoryGraph::new("x");
        let mut e1 = entity("e1", "exact", "thing");
        e1.embedding = vec![1.0, 0.0, 0.0];
        let mut e2 = entity("e2", "near", "thing");
        e2.embedding = vec![0.9, 0.1, 0.0];
        let mut e3 = entity("e3", "far", "thing");
        e3.embedding = vec![0.0, 1.0, 0.0];
        g.upsert_entity(&e1).await.unwrap();
        g.upsert_entity(&e2).await.unwrap();
        g.upsert_entity(&e3).await.unwrap();

        let hits = g
            .vector_search_entities(&[1.0, 0.0, 0.0], 2)
            .await
            .unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].0.id, "e1");
        assert_eq!(hits[1].0.id, "e2");
        // Scores monotonically descending.
        assert!(hits[0].1 >= hits[1].1);
    }

    // ---- Invalidation & deletion ----

    #[tokio::test]
    async fn invalidate_unknown_edge_id_is_silent() {
        let g = InMemoryGraph::new("x");
        // Documents current behaviour: invalid id is a no-op, not an error.
        g.invalidate_edge(9999, Utc::now()).await.unwrap();
    }

    #[tokio::test]
    async fn delete_entity_invalidates_incident_edges() {
        let g = populated_graph().await;
        let now = Utc::now();
        g.create_edge("a", "b", &relation("Alice knows Bob", "KNOWS", now))
            .await
            .unwrap();
        g.create_edge("a", "c", &relation("Alice at Acme", "WORKS_AT", now))
            .await
            .unwrap();
        let invalidated = g.delete_entity("a").await.unwrap();
        assert_eq!(invalidated, 2);
        assert!(g.get_entity_by_id("a").await.unwrap().is_none());
        // No active edges remain referring to a.
        assert!(g.find_all_active_edges_from("a").await.unwrap().is_empty());
    }

    // ---- TTL & memory tier ----

    #[tokio::test]
    async fn expire_ttl_edges_invalidates_edges_past_expiry() {
        let g = populated_graph().await;
        let now = Utc::now();
        let mut rel = relation("Alice knows Bob", "KNOWS", now);
        rel.expires_at = Some(now - Duration::seconds(1));
        let id = g.create_edge("a", "b", &rel).await.unwrap();
        let n = g.expire_ttl_edges(now).await.unwrap();
        assert_eq!(n, 1);
        // Edge is now invalidated.
        let edges = g.dump_all_edges().await.unwrap();
        let row = edges.iter().find(|e| e.edge_id == id).unwrap();
        assert!(row.invalid_at.is_some());
    }

    #[tokio::test]
    async fn memory_tier_stats_counts_only_active_edges() {
        let g = populated_graph().await;
        let now = Utc::now();
        let id = g
            .create_edge("a", "b", &relation("ab1", "KNOWS", now))
            .await
            .unwrap();
        g.create_edge("a", "c", &relation("ab2", "KNOWS", now)).await.unwrap();
        g.invalidate_edge(id, now).await.unwrap();
        let stats = g.memory_tier_stats().await.unwrap();
        assert_eq!(stats.working_count, 1);
        assert_eq!(stats.long_term_count, 0);
    }

    // ---- Properties ----

    #[tokio::test]
    async fn set_and_find_entity_by_property() {
        let g = populated_graph().await;
        g.set_entity_property("a", "email", "alice@example.com").await.unwrap();
        let found = g
            .find_entity_by_property("email", "alice@example.com")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, "a");
    }

    #[tokio::test]
    async fn find_entity_by_property_returns_none_when_absent() {
        let g = populated_graph().await;
        assert!(g
            .find_entity_by_property("email", "nobody@example.com")
            .await
            .unwrap()
            .is_none());
    }

    // ---- Compounding & supersession ----

    #[tokio::test]
    async fn compound_edge_confidence_updates_and_caps() {
        let g = populated_graph().await;
        let now = Utc::now();
        let id = g
            .create_edge("a", "b", &relation("Alice knows Bob", "KNOWS", now))
            .await
            .unwrap();
        let r1 = g.compound_edge_confidence(id, "agentX", 0.9).await.unwrap();
        // 1 - (1 - 0.9)*(1 - 0.9) = 0.99.
        assert!((r1 - 0.99).abs() < 1e-3);
        // New agent recorded.
        let edges = g.dump_all_edges().await.unwrap();
        let row = edges.iter().find(|e| e.edge_id == id).unwrap();
        assert!(row.source_agents.contains("agentX"));
    }

    #[tokio::test]
    async fn compound_edge_confidence_caps_at_0_99() {
        // Regression: previously the in-memory and SQL backends omitted the 0.99
        // cap that the FalkorDB backend applies. After centralising on
        // math::compound_confidence, all backends share the cap.
        let g = populated_graph().await;
        let now = Utc::now();
        let mut rel = relation("Alice knows Bob", "KNOWS", now);
        rel.confidence = 0.95;
        let id = g.create_edge("a", "b", &rel).await.unwrap();
        let r = g.compound_edge_confidence(id, "agentX", 0.95).await.unwrap();
        assert!(r <= 0.99 + 1e-6, "compounded confidence {} exceeded cap", r);
    }

    #[tokio::test]
    async fn compound_unknown_edge_returns_new_confidence() {
        let g = InMemoryGraph::new("x");
        let r = g.compound_edge_confidence(9999, "agent", 0.4).await.unwrap();
        assert!((r - 0.4).abs() < 1e-6);
    }

    #[tokio::test]
    async fn supersession_chain_round_trip() {
        let g = InMemoryGraph::new("x");
        let now = Utc::now();
        g.create_supersession(1, 2, now, "old", "new").await.unwrap();
        let chain = g.get_supersession_chain(1).await.unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].old_edge_id, 1);
        assert_eq!(chain[0].new_edge_id, 2);
    }

    // ---- Stats ----

    #[tokio::test]
    async fn graph_stats_counts_entities_and_active_edges() {
        let g = populated_graph().await;
        let now = Utc::now();
        g.create_edge("a", "b", &relation("ab", "KNOWS", now)).await.unwrap();
        let stats = g.graph_stats().await.unwrap();
        assert_eq!(stats.entity_count, 3);
        assert_eq!(stats.edge_count, 1);
    }
}
