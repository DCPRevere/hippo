//! Concrete Dreamer actions: Linker, Inferrer, Reconciler, Consolidator.
//!
//! Each is a Dreamer that processes one entity at a time. The pool drives
//! them; they query the graph for the entities that need their attention,
//! then write append-only facts (or, in the Reconciler's case, supersession
//! relationships).
//!
//! Append-only invariant: none of these actions delete or modify existing
//! facts. Reconciliation is expressed as a `supersedes` fact, not as
//! invalidation. The user-only `retract`/`correct` operations live
//! elsewhere (src/http/handlers/core.rs).

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;

use super::{DreamReport, Dreamer, WorkUnit};
use crate::graph_backend::GraphBackend;
use crate::math::cosine_similarity;
use crate::models::{MemoryTier, Relation};
use crate::state::AppState;

/// Discover links between entities that are semantically close but not yet
/// connected by an edge. The Dreamer's most distinctive action — finding
/// connections you didn't know were there.
pub struct Linker {
    state: Arc<AppState>,
}

impl Linker {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Dreamer for Linker {
    fn name(&self) -> &str {
        "linker"
    }

    async fn next_unit(&self, graph: &dyn GraphBackend) -> Result<Option<WorkUnit>> {
        super::next_unvisited_entity(graph, 100).await
    }

    async fn process(&self, graph: &dyn GraphBackend, unit: WorkUnit) -> Result<DreamReport> {
        let mut report = DreamReport {
            facts_visited: 1,
            ..DreamReport::default()
        };

        let entity = match graph.get_entity_by_id(&unit.entity_id).await? {
            Some(e) => e,
            None => return Ok(report),
        };

        let threshold = self
            .state
            .config
            .pipeline
            .tuning
            .link_discovery_cosine_threshold;
        let close = graph
            .find_close_unlinked(&unit.entity_id, &entity.embedding, threshold)
            .await?;

        for (candidate, _score) in close {
            // Per-pair dedup against AppState.checked_pairs (legacy cache).
            let pair = if unit.entity_id < candidate.id {
                (unit.entity_id.clone(), candidate.id.clone())
            } else {
                (candidate.id.clone(), unit.entity_id.clone())
            };
            {
                let checked = self.state.checked_pairs.read().await;
                if checked.contains(&pair) {
                    continue;
                }
            }

            let a_facts = graph.get_entity_facts(&unit.entity_id).await?;
            let b_facts = graph.get_entity_facts(&candidate.id).await?;

            if let Some((rel_type, fact, confidence)) = self
                .state
                .llm
                .discover_link(&entity, &candidate, &a_facts, &b_facts)
                .await?
            {
                let embedding = self.state.llm.embed(&fact).await?;
                let now = Utc::now();
                let relation = Relation {
                    fact,
                    relation_type: rel_type,
                    embedding,
                    source_agents: vec!["dreamer/linker".to_string()],
                    valid_at: now,
                    invalid_at: None,
                    confidence,
                    salience: 0,
                    created_at: now,
                    memory_tier: MemoryTier::Working,
                    expires_at: None,
                };
                graph
                    .create_edge(&unit.entity_id, &candidate.id, &relation)
                    .await?;
                report.links_written += 1;
            }

            // Mark pair as checked to avoid re-asking the LLM about it.
            let cache_max = self.state.config.pipeline.tuning.link_pair_cache_max;
            let cache_evict = self.state.config.pipeline.tuning.link_pair_cache_evict;
            let mut checked = self.state.checked_pairs.write().await;
            checked.insert(pair);
            if checked.len() > cache_max {
                let to_remove: Vec<_> = checked.iter().take(cache_evict).cloned().collect();
                for pair in to_remove {
                    checked.remove(&pair);
                }
            }
        }

        Ok(report)
    }
}

/// Detect contradictions between active edges on the same entity and
/// resolve them append-only by writing a `supersedes` relationship. Both
/// original facts remain in the graph; retrieval consults the supersession
/// to filter superseded edges. The Dreamer never invalidates.
pub struct Reconciler {
    state: Arc<AppState>,
}

impl Reconciler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Dreamer for Reconciler {
    fn name(&self) -> &str {
        "reconciler"
    }

    async fn next_unit(&self, graph: &dyn GraphBackend) -> Result<Option<WorkUnit>> {
        super::next_unvisited_entity(graph, 100).await
    }

    async fn process(&self, graph: &dyn GraphBackend, unit: WorkUnit) -> Result<DreamReport> {
        let mut report = DreamReport {
            facts_visited: 1,
            ..DreamReport::default()
        };

        let edges = graph.find_all_active_edges_from(&unit.entity_id).await?;

        // Group active edges by (object, relation_type). Multiple edges in
        // one group are candidate contradictions.
        let mut groups: std::collections::HashMap<(String, String), Vec<_>> =
            std::collections::HashMap::new();
        for edge in edges {
            groups
                .entry((edge.object_id.clone(), edge.relation_type.clone()))
                .or_default()
                .push(edge);
        }

        // Collect the sources whose claims got superseded so we can penalise
        // their credibility after we drop the read lock.
        let mut superseded_sources: Vec<String> = Vec::new();

        {
            let cred = self.state.credibility.read().await;

            for ((_, rel_type), group) in groups {
                if group.len() < 2 {
                    continue;
                }
                for i in 0..group.len() {
                    for j in (i + 1)..group.len() {
                        let (classification, _) = self
                            .state
                            .llm
                            .classify_edge(&group[i].fact, &group[j].fact, &rel_type)
                            .await?;

                        if classification != crate::models::EdgeClassification::Contradiction {
                            continue;
                        }

                        report.contradictions_seen += 1;

                        // Pick the older edge as the one being superseded, but
                        // weighted by source credibility: if the *newer* edge
                        // came from a less-credible source than the older one,
                        // skip — don't supersede with weak evidence.
                        let (older_idx, newer_idx) = if group[i].valid_at < group[j].valid_at {
                            (i, j)
                        } else {
                            (j, i)
                        };
                        let older = &group[older_idx];
                        let newer = &group[newer_idx];

                        let older_cred = older
                            .source_agents
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(|s| cred.get(s))
                            .fold(0.8f32, f32::max);
                        let newer_cred = newer
                            .source_agents
                            .split(',')
                            .filter(|s| !s.is_empty())
                            .map(|s| cred.get(s))
                            .fold(0.8f32, f32::max);

                        if newer_cred + 0.05 < older_cred {
                            // Skip: the new claim's source is meaningfully
                            // less credible than the old one's. Keep both
                            // active and let future evidence decide.
                            continue;
                        }

                        graph.supersede_edge(older.edge_id, newer.edge_id).await?;
                        report.supersessions_written += 1;

                        // The older edge's sources just had a claim
                        // overruled. Record one contradiction per
                        // distinct source so credibility compounds.
                        for source in older.source_agents.split(',') {
                            let s = source.trim();
                            if !s.is_empty() {
                                superseded_sources.push(s.to_string());
                            }
                        }
                    }
                }
            }
        }

        if !superseded_sources.is_empty() {
            let mut cred = self.state.credibility.write().await;
            for source in &superseded_sources {
                cred.record_contradiction(source);
            }
        }

        Ok(report)
    }
}

/// Walk an entity's 1-hop neighbourhood and ask the LLM what relationships
/// are implied by what's already known. Inferred edges are tagged in their
/// source_agents and have a confidence discount applied.
pub struct Inferrer {
    state: Arc<AppState>,
}

impl Inferrer {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Dreamer for Inferrer {
    fn name(&self) -> &str {
        "inferrer"
    }

    async fn next_unit(&self, graph: &dyn GraphBackend) -> Result<Option<WorkUnit>> {
        super::next_unvisited_entity(graph, 100).await
    }

    async fn process(&self, graph: &dyn GraphBackend, unit: WorkUnit) -> Result<DreamReport> {
        let mut report = DreamReport {
            facts_visited: 1,
            ..DreamReport::default()
        };

        let entity = match graph.get_entity_by_id(&unit.entity_id).await? {
            Some(e) => e,
            None => return Ok(report),
        };

        let entity_facts = graph.get_entity_facts(&unit.entity_id).await?;
        let hop_results = graph
            .walk_n_hops(std::slice::from_ref(&unit.entity_id), 1, 20, None)
            .await?;
        let hop_edges: Vec<_> = hop_results.into_iter().map(|(e, _)| e).collect();

        let mut neighbour_map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for edge in &hop_edges {
            let neighbour_name = if edge.subject_id == unit.entity_id {
                &edge.object_name
            } else {
                &edge.subject_name
            };
            neighbour_map
                .entry(neighbour_name.clone())
                .or_default()
                .push(edge.fact.clone());
        }
        let neighbor_facts: Vec<(String, Vec<String>)> = neighbour_map.into_iter().collect();

        let inferences = self
            .state
            .llm
            .find_missing_inferences(&entity.name, &entity_facts, &neighbor_facts)
            .await?;

        for (rel_type, object_name, fact_text, confidence) in inferences {
            let object_entities = graph.fulltext_search_entities(&object_name).await?;
            let object_id = object_entities
                .iter()
                .find(|e| e.name.to_lowercase() == object_name.to_lowercase())
                .map(|e| e.id.clone());

            let object_id = match object_id {
                Some(id) => id,
                None => continue,
            };

            // Embedding-based dedup: don't write if a near-identical fact
            // already exists.
            let embedding = self.state.llm.embed(&fact_text).await?;
            let existing = graph.find_all_active_edges_from(&unit.entity_id).await?;
            let dup_threshold = self.state.config.pipeline.tuning.duplicate_cosine_threshold;
            let is_duplicate = existing.iter().any(|e| {
                if e.embedding.is_empty() {
                    return false;
                }
                cosine_similarity(&embedding, &e.embedding) > dup_threshold
            });
            if is_duplicate {
                continue;
            }

            let now = Utc::now();
            let relation = Relation {
                fact: fact_text,
                relation_type: rel_type,
                embedding,
                source_agents: vec!["dreamer/inferrer".to_string()],
                valid_at: now,
                invalid_at: None,
                confidence: confidence * self.state.config.pipeline.tuning.inferred_fact_discount,
                salience: 0,
                created_at: now,
                memory_tier: MemoryTier::Working,
                expires_at: None,
            };
            graph
                .create_edge(&unit.entity_id, &object_id, &relation)
                .await?;
            report.inferences_written += 1;
        }

        Ok(report)
    }
}

/// Cluster episodic facts about a single entity within a recent time
/// window into a higher-order semantic-profile fact. Source episodes
/// remain queryable; the consolidated fact is a new edge linked back to
/// them via salience and edge metadata.
///
/// This is the brain-inspired action with no direct competitor in
/// Mem0/Zep/Supermemory. The first version is intentionally conservative:
/// it requires `min_facts_for_pattern` (configurable) before producing a
/// summary, and uses the LLM to write the summary. Future versions can
/// add multi-hop clustering and structural pattern matching.
pub struct Consolidator {
    state: Arc<AppState>,
}

impl Consolidator {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl Dreamer for Consolidator {
    fn name(&self) -> &str {
        "consolidator"
    }

    async fn next_unit(&self, graph: &dyn GraphBackend) -> Result<Option<WorkUnit>> {
        super::next_unvisited_entity(graph, 100).await
    }

    async fn process(&self, graph: &dyn GraphBackend, unit: WorkUnit) -> Result<DreamReport> {
        let mut report = DreamReport {
            facts_visited: 1,
            ..DreamReport::default()
        };

        let entity = match graph.get_entity_by_id(&unit.entity_id).await? {
            Some(e) => e,
            None => return Ok(report),
        };

        let edges = graph.find_all_active_edges_from(&unit.entity_id).await?;
        let min_facts = self.state.config.pipeline.tuning.consolidation_min_facts;

        if edges.len() < min_facts {
            return Ok(report);
        }

        // Filter to the working tier — episodic content that's a candidate
        // for consolidation. Long-term facts are already abstract enough.
        let episodic: Vec<_> = edges
            .iter()
            .filter(|e| e.memory_tier == "working")
            .cloned()
            .collect();
        if episodic.len() < min_facts {
            return Ok(report);
        }

        // Ask the LLM to summarise the recent episodic facts into a single
        // pattern. We reuse `discover_link` semantics minimally — produce
        // a relation_type + fact tuple — by phrasing the request through
        // the existing find_missing_inferences API, which already returns
        // (rel_type, object, fact, confidence).
        //
        // For now, the consolidation target is the entity itself (a
        // self-edge encoding the pattern): "Entity has been observed to
        // ...". Object is the entity's own name; consolidator-written
        // facts are tagged in source_agents.
        let entity_facts: Vec<String> = episodic.iter().map(|e| e.fact.clone()).collect();
        let neighbour_context: Vec<(String, Vec<String>)> = vec![];

        let inferences = self
            .state
            .llm
            .find_missing_inferences(&entity.name, &entity_facts, &neighbour_context)
            .await?;

        // Take the first inference as the consolidated fact. Future versions
        // can do real clustering rather than relying on the inference API.
        if let Some((rel_type, _object_name, fact_text, confidence)) = inferences.into_iter().next()
        {
            let embedding = self.state.llm.embed(&fact_text).await?;
            let now = Utc::now();
            let relation = Relation {
                fact: fact_text,
                relation_type: rel_type,
                embedding,
                source_agents: vec!["dreamer/consolidator".to_string()],
                valid_at: now,
                invalid_at: None,
                confidence: confidence * self.state.config.pipeline.tuning.inferred_fact_discount,
                salience: 0,
                created_at: now,
                memory_tier: MemoryTier::LongTerm,
                expires_at: None,
            };
            // Self-edge: the consolidated pattern is *about* the entity.
            graph
                .create_edge(&unit.entity_id, &unit.entity_id, &relation)
                .await?;
            report.consolidations_written += 1;
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::InMemoryGraph;
    use crate::config::Config;
    use crate::math::pseudo_embed;
    use crate::models::{EdgeClassification, Entity, MemoryTier};
    use crate::state::AppState;
    use crate::testing::FakeLlm;
    use chrono::Duration;

    async fn seed(graph: &InMemoryGraph, id: &str, name: &str) {
        graph
            .upsert_entity(&Entity {
                id: id.into(),
                name: name.into(),
                entity_type: "person".into(),
                resolved: true,
                hint: None,
                content: None,
                created_at: Utc::now(),
                embedding: pseudo_embed(name),
            })
            .await
            .unwrap();
    }

    fn rel(fact: &str, rel: &str, source: &str, age: Duration) -> Relation {
        Relation {
            fact: fact.into(),
            relation_type: rel.into(),
            embedding: pseudo_embed(fact),
            source_agents: vec![source.into()],
            valid_at: Utc::now() - age,
            invalid_at: None,
            confidence: 0.9,
            salience: 0,
            created_at: Utc::now() - age,
            memory_tier: MemoryTier::LongTerm,
            expires_at: None,
        }
    }

    /// Regression: when the Reconciler supersedes an edge because of a
    /// contradiction, it must also penalise the credibility of the source
    /// whose claim was overruled. Before this fix the dreamer wrote the
    /// supersession edge but never called `record_contradiction`, so
    /// repeat-offender sources kept full credibility.
    #[tokio::test]
    async fn reconciler_records_contradiction_against_superseded_source() {
        let fake = FakeLlm::new().with_classification(EdgeClassification::Contradiction, 0.95);
        let state = Arc::new(AppState::for_test(Arc::new(fake), Config::test_default()));
        let graph = InMemoryGraph::new("test");

        seed(&graph, "alice", "Alice").await;
        seed(&graph, "acme", "Acme").await;

        // Two edges between the same pair (alice → acme, WORKS_AT) with
        // contradicting fact text. The Reconciler groups by
        // (object_id, relation_type), so both end up in the same group and
        // get classified.
        graph
            .create_edge(
                "alice",
                "acme",
                &rel(
                    "Alice works at Acme as a lawyer",
                    "WORKS_AT",
                    "untrusted-bot",
                    Duration::days(2),
                ),
            )
            .await
            .unwrap();
        graph
            .create_edge(
                "alice",
                "acme",
                &rel(
                    "Alice works at Acme as a doctor",
                    "WORKS_AT",
                    "test-agent",
                    Duration::hours(1),
                ),
            )
            .await
            .unwrap();

        // Default credibility is 0.8 for an unknown source.
        let before = state.credibility.read().await.get("untrusted-bot");
        assert!((before - 0.8).abs() < 1e-6);

        let recon = Reconciler::new(state.clone());
        let report = recon
            .process(
                &graph,
                WorkUnit {
                    entity_id: "alice".into(),
                    score: 0.0,
                },
            )
            .await
            .unwrap();

        assert_eq!(report.contradictions_seen, 1);
        assert_eq!(report.supersessions_written, 1);

        // The superseded source must lose credibility.
        let after = state.credibility.read().await.get("untrusted-bot");
        assert!(
            after < before,
            "untrusted-bot credibility should fall after a contradiction; before={before}, after={after}"
        );
    }
}
