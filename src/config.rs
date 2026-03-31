use anyhow::{Context, Result};

pub enum AnthropicAuth {
    ApiKey(String),
    OAuthToken(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmProvider {
    Anthropic,
    OpenAI,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraphBackendType {
    FalkorDB,
    Memory,
}

pub struct Config {
    pub port: u16,
    pub graph_backend: GraphBackendType,
    pub falkordb_url: String,
    pub anthropic_auth: AnthropicAuth,
    pub anthropic_model: String,
    pub ollama_url: String,
    pub maintenance_interval_secs: u64,
    pub graph_name: String,
    pub fixture_mode: String,
    pub fixture_path: String,
    pub mock_llm: bool,
    pub allow_admin: bool,
    pub llm_provider: LlmProvider,
    pub openai_api_key: Option<String>,
    pub openai_base_url: String,
    pub openai_model: String,
    pub openai_embedding_model: Option<String>,
    pub infer_pre_context: bool,
    pub infer_enrichment: bool,
    pub infer_maintenance: bool,
    pub llm_max_tokens: u32,
    pub default_context_limit: usize,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let mock_llm = std::env::var("MOCK_LLM").map_or(false, |v| v == "1");

        let llm_provider = match std::env::var("LLM_PROVIDER").as_deref() {
            Ok("openai") => LlmProvider::OpenAI,
            _ => LlmProvider::Anthropic,
        };

        let anthropic_auth = if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            AnthropicAuth::ApiKey(key)
        } else if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
            AnthropicAuth::OAuthToken(token)
        } else if mock_llm || llm_provider == LlmProvider::OpenAI {
            AnthropicAuth::ApiKey("not-used".to_string())
        } else {
            anyhow::bail!("Either ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN is required");
        };

        let openai_api_key = std::env::var("OPENAI_API_KEY").ok();
        if llm_provider == LlmProvider::OpenAI && openai_api_key.is_none() && !mock_llm {
            anyhow::bail!("OPENAI_API_KEY is required when LLM_PROVIDER=openai");
        }

        let graph_backend = match std::env::var("GRAPH_BACKEND").as_deref() {
            Ok("memory") => GraphBackendType::Memory,
            _ => GraphBackendType::FalkorDB,
        };

        Ok(Config {
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "21693".to_string())
                .parse()
                .context("invalid PORT")?,
            graph_backend,
            falkordb_url: std::env::var("FALKORDB_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            anthropic_auth,
            anthropic_model: std::env::var("ANTHROPIC_MODEL")
                .unwrap_or_else(|_| "claude-haiku-4-5-20251001".to_string()),
            ollama_url: std::env::var("OLLAMA_URL")
                .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            maintenance_interval_secs: std::env::var("MAINTENANCE_INTERVAL_SECS")
                .unwrap_or_else(|_| "10".to_string())
                .parse()
                .context("invalid MAINTENANCE_INTERVAL_SECS")?,
            graph_name: std::env::var("GRAPH_NAME")
                .unwrap_or_else(|_| "hippo".to_string()),
            fixture_mode: std::env::var("EVAL_RECORD")
                .map(|_| "record".to_string())
                .or_else(|_| std::env::var("EVAL_REPLAY").map(|_| "replay".to_string()))
                .unwrap_or_else(|_| "none".to_string()),
            fixture_path: std::env::var("FIXTURE_PATH")
                .unwrap_or_else(|_| "./fixtures/llm-responses.json".to_string()),
            mock_llm,
            allow_admin: std::env::var("ALLOW_ADMIN").map_or(false, |v| v == "1"),
            llm_provider,
            openai_api_key,
            openai_base_url: std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
            openai_model: std::env::var("OPENAI_MODEL")
                .unwrap_or_else(|_| "gpt-4o-mini".to_string()),
            openai_embedding_model: std::env::var("OPENAI_EMBEDDING_MODEL").ok(),
            infer_pre_context: std::env::var("INFER_PRE_CONTEXT").map_or(false, |v| v == "1"),
            infer_enrichment: std::env::var("INFER_ENRICHMENT").map_or(false, |v| v == "1"),
            infer_maintenance: std::env::var("INFER_MAINTENANCE").map_or(false, |v| v == "1"),
            llm_max_tokens: std::env::var("LLM_MAX_TOKENS")
                .unwrap_or_else(|_| "32768".to_string())
                .parse()
                .context("invalid LLM_MAX_TOKENS")?,
            default_context_limit: std::env::var("DEFAULT_CONTEXT_LIMIT")
                .unwrap_or_else(|_| "50".to_string())
                .parse()
                .context("invalid DEFAULT_CONTEXT_LIMIT")?,
        })
    }

    pub fn falkordb_connection_string(&self) -> String {
        self.falkordb_url.replace("redis://", "falkor://")
    }

    /// Configuration suitable for unit tests (no env vars required).
    pub fn test_default() -> Self {
        Config {
            port: 0,
            graph_backend: GraphBackendType::Memory,
            falkordb_url: String::new(),
            anthropic_auth: AnthropicAuth::ApiKey("test-key".to_string()),
            anthropic_model: "test-model".to_string(),
            ollama_url: String::new(),
            maintenance_interval_secs: 3600,
            graph_name: "test".to_string(),
            fixture_mode: "none".to_string(),
            fixture_path: String::new(),
            mock_llm: false,
            allow_admin: true,
            llm_provider: LlmProvider::Anthropic,
            openai_api_key: None,
            openai_base_url: String::new(),
            openai_model: String::new(),
            openai_embedding_model: None,
            infer_pre_context: false,
            infer_enrichment: false,
            infer_maintenance: false,
            llm_max_tokens: 4096,
            default_context_limit: 50,
        }
    }
}
