use anyhow::{Context, Result};
use serde::Deserialize;

use crate::models::ScoringParams;

const DEFAULT_CONFIG_PATH: &str = "hippo.toml";

// --- Secrets (env-var only, never serialised) ---

pub enum AnthropicAuth {
    ApiKey(String),
    OAuthToken(String),
}

impl std::fmt::Debug for AnthropicAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiKey(_) => f.write_str("ApiKey(****)"),
            Self::OAuthToken(_) => f.write_str("OAuthToken(****)"),
        }
    }
}

impl Clone for AnthropicAuth {
    fn clone(&self) -> Self {
        match self {
            Self::ApiKey(k) => Self::ApiKey(k.clone()),
            Self::OAuthToken(t) => Self::OAuthToken(t.clone()),
        }
    }
}

// --- Serde-friendly enums ---

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Anthropic,
    #[serde(alias = "openai")]
    OpenAI,
}

impl Default for LlmProvider {
    fn default() -> Self {
        Self::Anthropic
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphBackendType {
    FalkorDB,
    Memory,
    Sqlite,
}

impl Default for GraphBackendType {
    fn default() -> Self {
        Self::Memory
    }
}

// --- Sub-configs ---

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    pub backend: GraphBackendType,
    pub name: String,
    pub sqlite: SqliteConfig,
    pub falkordb: FalkorDbConfig,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            backend: GraphBackendType::default(),
            name: "hippo".to_string(),
            sqlite: SqliteConfig::default(),
            falkordb: FalkorDbConfig::default(),
        }
    }
}

impl GraphConfig {
    pub fn falkordb_connection_string(&self) -> String {
        self.falkordb.url.replace("redis://", "falkor://")
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SqliteConfig {
    pub path: String,
}

impl Default for SqliteConfig {
    fn default() -> Self {
        Self {
            path: "hippo.db".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FalkorDbConfig {
    pub url: String,
}

impl Default for FalkorDbConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AnthropicConfig {
    pub model: String,
}

impl Default for AnthropicConfig {
    fn default() -> Self {
        Self {
            model: "claude-haiku-4-5-20251001".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OpenAiConfig {
    pub base_url: String,
    pub model: String,
    pub embedding_model: Option<String>,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
            embedding_model: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub url: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:11434".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    #[serde(rename = "mock")]
    pub mock_llm: bool,
    pub max_tokens: u32,
    pub extraction_prompt: String,
    pub anthropic: AnthropicConfig,
    pub openai: OpenAiConfig,
    pub ollama: OllamaConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::default(),
            mock_llm: false,
            max_tokens: 32768,
            extraction_prompt: String::new(),
            anthropic: AnthropicConfig::default(),
            openai: OpenAiConfig::default(),
            ollama: OllamaConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PipelineConfig {
    pub default_context_limit: usize,
    pub default_ttl_secs: Option<u64>,
    pub maintenance_interval_secs: u64,
    pub infer_pre_context: bool,
    pub infer_enrichment: bool,
    pub infer_maintenance: bool,
    pub scoring: ScoringParams,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            default_context_limit: 50,
            default_ttl_secs: None,
            maintenance_interval_secs: 10,
            infer_pre_context: false,
            infer_enrichment: false,
            infer_maintenance: false,
            scoring: ScoringParams::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AuthConfig {
    pub enabled: bool,
    pub insecure: bool,
    pub allow_admin: bool,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            insecure: false,
            allow_admin: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct EvalConfig {
    pub fixture_mode: String,
    pub fixture_path: String,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            fixture_mode: "none".to_string(),
            fixture_path: "./fixtures/llm-responses.json".to_string(),
        }
    }
}

// --- Top-level Config ---

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub port: u16,
    pub graph: GraphConfig,
    pub llm: LlmConfig,
    pub pipeline: PipelineConfig,
    pub auth: AuthConfig,
    pub eval: EvalConfig,

    // Secrets — env-var only, never in TOML.
    #[serde(skip)]
    pub anthropic_auth: Option<AnthropicAuth>,
    #[serde(skip)]
    pub openai_api_key: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: 21693,
            graph: GraphConfig::default(),
            llm: LlmConfig::default(),
            pipeline: PipelineConfig::default(),
            auth: AuthConfig::default(),
            eval: EvalConfig::default(),
            anthropic_auth: None,
            openai_api_key: None,
        }
    }
}

impl Config {
    /// Load configuration: TOML file (if present) -> env var overrides -> secret resolution.
    pub fn load() -> Result<Self> {
        let config_path = std::env::var("HIPPO_CONFIG")
            .unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());

        let mut config: Config = match std::fs::read_to_string(&config_path) {
            Ok(contents) => toml::from_str(&contents)
                .with_context(|| format!("failed to parse {config_path}"))?,
            Err(_) => Config::default(),
        };

        // --- Env var overrides (backward compat + CI/test support) ---

        if let Ok(v) = std::env::var("PORT") {
            config.port = v.parse().context("invalid PORT")?;
        }

        // Graph
        if let Ok(v) = std::env::var("GRAPH_BACKEND") {
            config.graph.backend = match v.as_str() {
                "falkordb" => GraphBackendType::FalkorDB,
                "sqlite" => GraphBackendType::Sqlite,
                _ => GraphBackendType::Memory,
            };
        }
        if let Ok(v) = std::env::var("GRAPH_NAME") {
            config.graph.name = v;
        }
        if let Ok(v) = std::env::var("FALKORDB_URL") {
            config.graph.falkordb.url = v;
        }
        if let Ok(v) = std::env::var("SQLITE_PATH") {
            config.graph.sqlite.path = v;
        }

        // LLM
        if let Ok(v) = std::env::var("LLM_PROVIDER") {
            config.llm.provider = match v.as_str() {
                "openai" => LlmProvider::OpenAI,
                _ => LlmProvider::Anthropic,
            };
        }
        if let Ok(v) = std::env::var("MOCK_LLM") {
            config.llm.mock_llm = v == "1";
        }
        if let Ok(v) = std::env::var("LLM_MAX_TOKENS") {
            config.llm.max_tokens = v.parse().context("invalid LLM_MAX_TOKENS")?;
        }
        if let Ok(v) = std::env::var("EXTRACTION_PROMPT") {
            config.llm.extraction_prompt = v;
        }
        if let Ok(v) = std::env::var("ANTHROPIC_MODEL") {
            config.llm.anthropic.model = v;
        }
        if let Ok(v) = std::env::var("OPENAI_BASE_URL") {
            config.llm.openai.base_url = v;
        }
        if let Ok(v) = std::env::var("OPENAI_MODEL") {
            config.llm.openai.model = v;
        }
        if let Ok(v) = std::env::var("OPENAI_EMBEDDING_MODEL") {
            config.llm.openai.embedding_model = Some(v);
        }
        if let Ok(v) = std::env::var("OLLAMA_URL") {
            config.llm.ollama.url = v;
        }

        // Pipeline
        if let Ok(v) = std::env::var("DEFAULT_CONTEXT_LIMIT") {
            config.pipeline.default_context_limit = v.parse().context("invalid DEFAULT_CONTEXT_LIMIT")?;
        }
        if let Ok(v) = std::env::var("DEFAULT_TTL_SECS") {
            config.pipeline.default_ttl_secs = Some(v.parse().context("invalid DEFAULT_TTL_SECS")?);
        }
        if let Ok(v) = std::env::var("MAINTENANCE_INTERVAL_SECS") {
            config.pipeline.maintenance_interval_secs = v.parse().context("invalid MAINTENANCE_INTERVAL_SECS")?;
        }
        if let Ok(v) = std::env::var("INFER_PRE_CONTEXT") {
            config.pipeline.infer_pre_context = v == "1";
        }
        if let Ok(v) = std::env::var("INFER_ENRICHMENT") {
            config.pipeline.infer_enrichment = v == "1";
        }
        if let Ok(v) = std::env::var("INFER_MAINTENANCE") {
            config.pipeline.infer_maintenance = v == "1";
        }
        if let Ok(v) = std::env::var("SCORING_W_RELEVANCE") {
            config.pipeline.scoring.w_relevance = v.parse().context("invalid SCORING_W_RELEVANCE")?;
        }
        if let Ok(v) = std::env::var("SCORING_W_CONFIDENCE") {
            config.pipeline.scoring.w_confidence = v.parse().context("invalid SCORING_W_CONFIDENCE")?;
        }
        if let Ok(v) = std::env::var("SCORING_W_RECENCY") {
            config.pipeline.scoring.w_recency = v.parse().context("invalid SCORING_W_RECENCY")?;
        }
        if let Ok(v) = std::env::var("SCORING_W_SALIENCE") {
            config.pipeline.scoring.w_salience = v.parse().context("invalid SCORING_W_SALIENCE")?;
        }
        if let Ok(v) = std::env::var("SCORING_MMR_LAMBDA") {
            config.pipeline.scoring.mmr_lambda = v.parse().context("invalid SCORING_MMR_LAMBDA")?;
        }

        // Auth
        if let Ok(v) = std::env::var("HIPPO_AUTH") {
            config.auth.enabled = v == "1";
        }
        if let Ok(v) = std::env::var("HIPPO_INSECURE") {
            config.auth.insecure = v == "1";
        }
        if let Ok(v) = std::env::var("ALLOW_ADMIN") {
            config.auth.allow_admin = v == "1";
        }

        // Eval / fixtures (legacy two-variable pattern preserved)
        if std::env::var("EVAL_RECORD").is_ok() {
            config.eval.fixture_mode = "record".to_string();
        } else if std::env::var("EVAL_REPLAY").is_ok() {
            config.eval.fixture_mode = "replay".to_string();
        }
        if let Ok(v) = std::env::var("FIXTURE_PATH") {
            config.eval.fixture_path = v;
        }

        // --- Secrets (always from env) ---

        config.openai_api_key = std::env::var("OPENAI_API_KEY").ok();

        config.anthropic_auth = Some(
            if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                AnthropicAuth::ApiKey(key)
            } else if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
                AnthropicAuth::OAuthToken(token)
            } else if config.llm.mock_llm || config.llm.provider == LlmProvider::OpenAI {
                AnthropicAuth::ApiKey("not-used".to_string())
            } else {
                anyhow::bail!("Either ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN is required");
            },
        );

        // --- Validation ---

        if config.llm.provider == LlmProvider::OpenAI
            && config.openai_api_key.is_none()
            && !config.llm.mock_llm
        {
            anyhow::bail!("OPENAI_API_KEY is required when llm.provider = \"openai\"");
        }

        Ok(config)
    }

    /// Configuration suitable for unit tests (no env vars or TOML file required).
    pub fn test_default() -> Self {
        Config {
            port: 0,
            graph: GraphConfig {
                name: "test".to_string(),
                ..Default::default()
            },
            llm: LlmConfig {
                max_tokens: 4096,
                anthropic: AnthropicConfig {
                    model: "test-model".to_string(),
                },
                ..Default::default()
            },
            pipeline: PipelineConfig {
                maintenance_interval_secs: 3600,
                ..Default::default()
            },
            auth: AuthConfig {
                allow_admin: true,
                ..Default::default()
            },
            eval: EvalConfig::default(),
            anthropic_auth: Some(AnthropicAuth::ApiKey("test-key".to_string())),
            openai_api_key: None,
        }
    }
}
