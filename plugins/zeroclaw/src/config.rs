/// Configuration for connecting to a Hippo instance.
#[derive(Debug, Clone)]
pub struct HippoConfig {
    /// Base URL for the Hippo HTTP API (default: `http://localhost:21693`).
    pub base_url: String,
    /// Optional API key for authenticated Hippo instances.
    pub api_key: Option<String>,
    /// Optional graph namespace. When set, all operations target this graph.
    pub graph: Option<String>,
}

impl Default for HippoConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:21693".to_string(),
            api_key: None,
            graph: None,
        }
    }
}

impl HippoConfig {
    /// Build a config from environment variables:
    /// - `HIPPO_URL` (default `http://localhost:21693`)
    /// - `HIPPO_API_KEY` (optional)
    /// - `HIPPO_GRAPH` (optional)
    pub fn from_env() -> Self {
        Self {
            base_url: std::env::var("HIPPO_URL")
                .unwrap_or_else(|_| "http://localhost:21693".to_string()),
            api_key: std::env::var("HIPPO_API_KEY").ok(),
            graph: std::env::var("HIPPO_GRAPH").ok(),
        }
    }
}
