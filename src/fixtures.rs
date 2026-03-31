use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmFixture {
    pub request_hash: String,
    pub system: String,
    pub user: String,
    pub response: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FixtureStore {
    pub fixtures: HashMap<String, LlmFixture>,
}

impl FixtureStore {
    pub fn load(path: &std::path::Path) -> Self {
        if path.exists() {
            let json = std::fs::read_to_string(path).unwrap_or_default();
            serde_json::from_str(&json).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_default();
        let _ = std::fs::write(path, json);
    }

    pub fn get(&self, hash: &str) -> Option<&LlmFixture> {
        self.fixtures.get(hash)
    }

    pub fn insert(&mut self, fixture: LlmFixture) {
        self.fixtures.insert(fixture.request_hash.clone(), fixture);
    }
}

pub fn hash_request(system: &str, user: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(system.as_bytes());
    hasher.update(b"\n---\n");
    hasher.update(user.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..16])
}
