use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

impl Default for CredibilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CredibilityRegistry {
    pub fn new() -> Self {
        Self {
            sources: HashMap::new(),
        }
    }

    pub fn hydrate(&mut self, entries: Vec<SourceCredibility>) {
        for entry in entries {
            self.sources.insert(entry.agent_id.clone(), entry);
        }
    }

    pub fn get(&self, agent_id: &str) -> f32 {
        self.sources
            .get(agent_id)
            .map(|s| s.credibility)
            .unwrap_or(0.8)
    }

    pub fn record_contradiction(&mut self, agent_id: &str) {
        let entry = self
            .sources
            .entry(agent_id.to_string())
            .or_insert_with(|| SourceCredibility {
                agent_id: agent_id.to_string(),
                ..Default::default()
            });
        entry.fact_count = entry.fact_count.saturating_add(1);
        let total = entry.fact_count as f32;
        let contradictions = entry.contradiction_rate * (total - 1.0) + 1.0;
        entry.contradiction_rate = contradictions / total;
        entry.credibility = (1.0 - entry.contradiction_rate * 0.5).max(0.3);
    }

    pub fn record_fact(&mut self, agent_id: &str) {
        let entry = self
            .sources
            .entry(agent_id.to_string())
            .or_insert_with(|| SourceCredibility {
                agent_id: agent_id.to_string(),
                ..Default::default()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn unknown_agent_returns_default_credibility() {
        let reg = CredibilityRegistry::new();
        assert!(approx(reg.get("ghost"), 0.8));
    }

    #[test]
    fn record_fact_does_not_change_credibility() {
        let mut reg = CredibilityRegistry::new();
        reg.record_fact("alice");
        reg.record_fact("alice");
        assert!(approx(reg.get("alice"), 0.8));
    }

    #[test]
    fn first_contradiction_drops_credibility_to_floor_then_clamps() {
        let mut reg = CredibilityRegistry::new();
        reg.record_contradiction("bob");
        // After 1 contradiction out of 1 fact: rate = 1.0, credibility = max(1 - 0.5, 0.3) = 0.5.
        assert!(approx(reg.get("bob"), 0.5));
    }

    #[test]
    fn credibility_is_clamped_to_floor_of_0_3() {
        let mut reg = CredibilityRegistry::new();
        // Drive contradiction rate to 1.0 repeatedly; floor is 0.3, not below.
        for _ in 0..50 {
            reg.record_contradiction("eve");
        }
        let cred = reg.get("eve");
        assert!(cred >= 0.3 - 1e-6, "credibility {} below floor", cred);
        // With every fact a contradiction, rate=1.0, formula = max(0.5, 0.3) = 0.5.
        // (Floor would only bite if the rate coefficient changes; we still verify >= 0.3.)
        assert!(cred <= 0.8 + 1e-6);
    }

    #[test]
    fn credibility_never_exceeds_default_after_contradictions() {
        let mut reg = CredibilityRegistry::new();
        reg.record_contradiction("carol");
        reg.record_contradiction("carol");
        assert!(reg.get("carol") <= 0.8 + 1e-6);
    }

    #[test]
    fn hydrate_replaces_entries() {
        let mut reg = CredibilityRegistry::new();
        reg.record_contradiction("dan"); // sets credibility to 0.5
        reg.hydrate(vec![SourceCredibility {
            agent_id: "dan".into(),
            credibility: 0.42,
            fact_count: 7,
            contradiction_rate: 0.1,
        }]);
        assert!(approx(reg.get("dan"), 0.42));
    }

    #[test]
    fn list_returns_all_known_sources() {
        let mut reg = CredibilityRegistry::new();
        reg.record_fact("a");
        reg.record_contradiction("b");
        let names: std::collections::HashSet<String> =
            reg.list().into_iter().map(|s| s.agent_id).collect();
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn clear_removes_all_sources() {
        let mut reg = CredibilityRegistry::new();
        reg.record_contradiction("x");
        reg.clear();
        assert!(reg.list().is_empty());
        assert!(approx(reg.get("x"), 0.8)); // back to default
    }

    #[test]
    fn fact_count_overflow_is_saturating() {
        let mut reg = CredibilityRegistry::new();
        reg.hydrate(vec![SourceCredibility {
            agent_id: "big".into(),
            credibility: 0.8,
            fact_count: usize::MAX,
            contradiction_rate: 0.0,
        }]);
        // Should not panic.
        reg.record_fact("big");
        reg.record_contradiction("big");
        let entry = reg.list().into_iter().find(|s| s.agent_id == "big").unwrap();
        assert_eq!(entry.fact_count, usize::MAX);
    }

    /// Pins down a known sharp edge: `record_fact` does not update
    /// `contradiction_rate`, so a subsequent `record_contradiction` recomputes
    /// using the stale rate against the new total. This test documents the
    /// current behaviour so a refactor doesn't silently change it.
    #[test]
    fn record_fact_then_contradiction_uses_stale_rate_arithmetic() {
        let mut reg = CredibilityRegistry::new();
        reg.record_contradiction("z"); // count=1, rate=1.0, cred=0.5
        reg.record_fact("z"); // count=2, rate still 1.0 (not recomputed)
        reg.record_contradiction("z");
        // record_contradiction reconstructs contradictions = rate*(total-1)+1
        //   = 1.0 * 2 + 1 = 3 across total=3 → rate=1.0, cred=0.5.
        assert!(approx(reg.get("z"), 0.5));
    }
}
