use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceCredibility {
    pub agent_id: String,
    pub credibility: f32,
    pub fact_count: usize,
    pub contradiction_rate: f32,
}

impl Default for SourceCredibility {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            credibility: 0.8,
            fact_count: 0,
            contradiction_rate: 0.0,
        }
    }
}

pub struct CredibilityRegistry {
    sources: HashMap<String, SourceCredibility>,
}

impl CredibilityRegistry {
    pub fn new() -> Self {
        Self { sources: HashMap::new() }
    }

    pub fn hydrate(&mut self, entries: Vec<SourceCredibility>) {
        for entry in entries {
            self.sources.insert(entry.agent_id.clone(), entry);
        }
    }

    pub fn get(&self, agent_id: &str) -> f32 {
        self.sources.get(agent_id).map(|s| s.credibility).unwrap_or(0.8)
    }

    pub fn record_contradiction(&mut self, agent_id: &str) {
        let entry = self.sources.entry(agent_id.to_string()).or_insert_with(|| {
            SourceCredibility {
                agent_id: agent_id.to_string(),
                ..Default::default()
            }
        });
        entry.fact_count = entry.fact_count.saturating_add(1);
        let total = entry.fact_count as f32;
        let contradictions = entry.contradiction_rate * (total - 1.0) + 1.0;
        entry.contradiction_rate = contradictions / total;
        entry.credibility = (1.0 - entry.contradiction_rate * 0.5).max(0.3);
    }

    pub fn record_fact(&mut self, agent_id: &str) {
        let entry = self.sources.entry(agent_id.to_string()).or_insert_with(|| {
            SourceCredibility {
                agent_id: agent_id.to_string(),
                ..Default::default()
            }
        });
        entry.fact_count = entry.fact_count.saturating_add(1);
    }

    pub fn list(&self) -> Vec<SourceCredibility> {
        self.sources.values().cloned().collect()
    }

    pub fn clear(&mut self) {
        self.sources.clear();
    }
}
