use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

use crate::models::{EdgeRow, Entity, EntityRow, GraphStats, MemoryTierStats, ProvenanceResponse, Relation, UnderDocumentedEntity};

#[async_trait]
pub trait GraphBackend: Send + Sync {
    fn graph_name(&self) -> &str;
    async fn ping(&self) -> Result<()>;
    async fn setup_schema(&self) -> Result<()>;
    async fn drop_and_reinitialise(&self) -> Result<()>;

    // --- Entity search ---
    async fn fulltext_search_entities(&self, query_str: &str) -> Result<Vec<EntityRow>>;
    async fn vector_search_entities(&self, embedding: &[f32], k: usize) -> Result<Vec<(EntityRow, f32)>>;
    async fn get_entity_by_id(&self, entity_id: &str) -> Result<Option<EntityRow>>;

    // --- Edge search ---
    async fn fulltext_search_edges(&self, query_str: &str, at: Option<DateTime<Utc>>) -> Result<Vec<EdgeRow>>;
    async fn vector_search_edges_scored(&self, embedding: &[f32], k: usize, at: Option<DateTime<Utc>>) -> Result<Vec<(EdgeRow, f32)>>;

    // --- Graph traversal ---
    async fn walk_n_hops(&self, seed_entity_ids: &[String], max_hops: usize, limit_per_hop: usize, at: Option<DateTime<Utc>>) -> Result<Vec<(EdgeRow, usize)>>;
    async fn entity_timeline(&self, entity_name: &str) -> Result<Vec<EdgeRow>>;
    async fn find_all_active_edges_from(&self, node_id: &str) -> Result<Vec<EdgeRow>>;

    // --- Mutation ---
    async fn upsert_entity(&self, entity: &Entity) -> Result<()>;
    async fn create_edge(&self, from_id: &str, to_id: &str, rel: &Relation) -> Result<i64>;
    async fn invalidate_edge(&self, edge_id: i64, at: DateTime<Utc>) -> Result<()>;
    async fn increment_salience(&self, edge_ids: &[i64]) -> Result<()>;
    async fn merge_placeholder(&self, placeholder_id: &str, resolved_id: &str) -> Result<()>;
    async fn delete_entity(&self, entity_id: &str) -> Result<usize>;

    // --- Memory tier management ---
    async fn promote_working_memory(&self) -> Result<usize>;
    async fn memory_tier_stats(&self) -> Result<MemoryTierStats>;
    async fn decay_stale_edges(&self, stale_before: DateTime<Utc>, now: DateTime<Utc>) -> Result<usize>;
    async fn expire_ttl_edges(&self, now: DateTime<Utc>) -> Result<usize>;

    // --- Facts / reflection ---
    async fn entity_facts(&self, entity_name: &str) -> Result<Vec<EdgeRow>>;
    async fn get_entity_facts(&self, entity_id: &str) -> Result<Vec<String>>;
    async fn graph_stats(&self) -> Result<GraphStats>;
    async fn all_relation_types(&self) -> Result<Vec<String>>;
    async fn under_documented_entities(&self, threshold: usize) -> Result<Vec<UnderDocumentedEntity>>;
    async fn entity_type_counts(&self) -> Result<HashMap<String, usize>>;

    // --- Dump / pagination ---
    async fn dump_all_entities(&self) -> Result<Vec<EntityRow>>;
    async fn dump_all_edges(&self) -> Result<Vec<EdgeRow>>;
    async fn list_entities_by_recency(&self, offset: usize, limit: usize) -> Result<Vec<EntityRow>>;

    // --- Supersession / provenance ---
    async fn get_provenance(&self, edge_id: i64) -> Result<ProvenanceResponse>;

    // --- Discovery ---
    async fn find_close_unlinked(&self, node_id: &str, embedding: &[f32], threshold: f32) -> Result<Vec<(EntityRow, f32)>>;
    async fn find_placeholder_nodes(&self, cutoff: DateTime<Utc>) -> Result<Vec<EntityRow>>;
    async fn find_two_hop_unlinked_pairs(&self, limit: usize) -> Result<Vec<(EntityRow, EntityRow)>>;

    // --- Archive ---
    async fn archive_low_confidence_edges(&self, threshold: f32, dry_run: bool) -> Result<Vec<EdgeRow>>;

    // --- Entity updates ---
    async fn rename_entity(&self, entity_id: &str, new_name: &str) -> Result<()>;
    async fn set_entity_property(&self, entity_id: &str, key: &str, value: &str) -> Result<()>;
    async fn find_entity_by_property(&self, key: &str, value: &str) -> Result<Option<EntityRow>>;

    // --- Clustering ---
    async fn find_entity_clusters(&self, min_size: usize) -> Result<Vec<Vec<String>>>;
}
