#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
use serde::Deserialize;

use crate::models::{PipelineTuning, ScoringParams};

#[cfg(not(target_arch = "wasm32"))]
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
#[derive(Default)]
pub enum LlmProvider {
    #[default]
    Anthropic,
    #[serde(alias = "openai")]
    OpenAI,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum GraphBackendType {
    FalkorDB,
    #[default]
    Memory,
    Postgres,
    Qdrant,
    Sqlite,
}

// --- Sub-configs ---

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    pub backend: GraphBackendType,
    pub name: String,
    pub sqlite: SqliteConfig,
    pub falkordb: FalkorDbConfig,
    pub postgres: PostgresConfig,
    pub qdrant: QdrantConfig,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            backend: GraphBackendType::default(),
            name: "hippo".to_string(),
            sqlite: SqliteConfig::default(),
            falkordb: FalkorDbConfig::default(),
            postgres: PostgresConfig::default(),
            qdrant: QdrantConfig::default(),
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
pub struct PostgresConfig {
    pub url: String,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            url: "postgres://localhost/hippo".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct QdrantConfig {
    pub url: String,
}

impl Default for QdrantConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:6334".to_string(),
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
            model: "gpt-5.4-mini".to_string(),
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
    pub tuning: PipelineTuning,
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
            tuning: PipelineTuning::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct AuthConfig {
    pub enabled: bool,
    pub insecure: bool,
    pub allow_admin: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub requests_per_minute: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_minute: 120,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct TlsConfig {
    pub enabled: bool,
    pub cert_path: String,
    pub key_path: String,
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
    pub rate_limit: RateLimitConfig,
    pub tls: TlsConfig,
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
            rate_limit: RateLimitConfig::default(),
            tls: TlsConfig::default(),
            eval: EvalConfig::default(),
            anthropic_auth: None,
            openai_api_key: None,
        }
    }
}

impl Config {
    /// Create a Config suitable for WASM (browser) usage.
    pub fn for_wasm(
        openai_api_key: String,
        model: Option<String>,
        embedding_model: Option<String>,
    ) -> Self {
        Config {
            graph: GraphConfig {
                backend: GraphBackendType::Memory,
                ..Default::default()
            },
            llm: LlmConfig {
                provider: LlmProvider::OpenAI,
                openai: OpenAiConfig {
                    model: model.unwrap_or_else(|| "gpt-5.4-mini".to_string()),
                    embedding_model: Some(
                        embedding_model.unwrap_or_else(|| "text-embedding-3-small".to_string()),
                    ),
                    ..Default::default()
                },
                ..Default::default()
            },
            pipeline: PipelineConfig {
                infer_pre_context: true,
                default_context_limit: 50,
                ..Default::default()
            },
            openai_api_key: Some(openai_api_key),
            ..Default::default()
        }
    }

    /// Load configuration: TOML file (if present) -> env var overrides -> secret resolution.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load() -> Result<Self> {
        let config_path =
            std::env::var("HIPPO_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG_PATH.to_string());

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
                "postgres" => GraphBackendType::Postgres,
                "qdrant" => GraphBackendType::Qdrant,
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
        if let Ok(v) = std::env::var("POSTGRES_URL") {
            config.graph.postgres.url = v;
        }
        if let Ok(v) = std::env::var("QDRANT_URL") {
            config.graph.qdrant.url = v;
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
            config.pipeline.default_context_limit =
                v.parse().context("invalid DEFAULT_CONTEXT_LIMIT")?;
        }
        if let Ok(v) = std::env::var("DEFAULT_TTL_SECS") {
            config.pipeline.default_ttl_secs = Some(v.parse().context("invalid DEFAULT_TTL_SECS")?);
        }
        if let Ok(v) = std::env::var("MAINTENANCE_INTERVAL_SECS") {
            config.pipeline.maintenance_interval_secs =
                v.parse().context("invalid MAINTENANCE_INTERVAL_SECS")?;
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
            config.pipeline.scoring.w_relevance =
                v.parse().context("invalid SCORING_W_RELEVANCE")?;
        }
        if let Ok(v) = std::env::var("SCORING_W_CONFIDENCE") {
            config.pipeline.scoring.w_confidence =
                v.parse().context("invalid SCORING_W_CONFIDENCE")?;
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

        // Rate limiting
        if let Ok(v) = std::env::var("HIPPO_RATE_LIMIT") {
            config.rate_limit.enabled = v == "1";
        }
        if let Ok(v) = std::env::var("HIPPO_RPM") {
            config.rate_limit.requests_per_minute = v.parse().context("invalid HIPPO_RPM")?;
        }

        // TLS
        if let Ok(v) = std::env::var("HIPPO_TLS") {
            config.tls.enabled = v == "1";
        }
        if let Ok(v) = std::env::var("HIPPO_TLS_CERT") {
            config.tls.cert_path = v;
        }
        if let Ok(v) = std::env::var("HIPPO_TLS_KEY") {
            config.tls.key_path = v;
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

        config.anthropic_auth = Some(if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            AnthropicAuth::ApiKey(key)
        } else if let Ok(token) = std::env::var("ANTHROPIC_OAUTH_TOKEN") {
            AnthropicAuth::OAuthToken(token)
        } else if config.llm.mock_llm || config.llm.provider == LlmProvider::OpenAI {
            AnthropicAuth::ApiKey("not-used".to_string())
        } else {
            anyhow::bail!("Either ANTHROPIC_API_KEY or ANTHROPIC_OAUTH_TOKEN is required");
        });

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
            rate_limit: RateLimitConfig::default(),
            tls: TlsConfig::default(),
            eval: EvalConfig::default(),
            anthropic_auth: Some(AnthropicAuth::ApiKey("test-key".to_string())),
            openai_api_key: None,
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    // ---- Defaults ----

    #[test]
    fn default_port_matches_published_value() {
        assert_eq!(Config::default().port, 21693);
    }

    #[test]
    fn default_graph_is_memory_named_hippo() {
        let cfg = Config::default();
        assert_eq!(cfg.graph.backend, GraphBackendType::Memory);
        assert_eq!(cfg.graph.name, "hippo");
    }

    #[test]
    fn default_llm_provider_is_anthropic_no_mock() {
        let cfg = Config::default();
        assert_eq!(cfg.llm.provider, LlmProvider::Anthropic);
        assert!(!cfg.llm.mock_llm);
    }

    #[test]
    fn default_secrets_are_unset() {
        let cfg = Config::default();
        assert!(cfg.anthropic_auth.is_none());
        assert!(cfg.openai_api_key.is_none());
    }

    #[test]
    fn default_rate_limit_disabled_with_120_rpm() {
        let cfg = Config::default();
        assert!(!cfg.rate_limit.enabled);
        assert_eq!(cfg.rate_limit.requests_per_minute, 120);
    }

    #[test]
    fn falkordb_connection_string_swaps_scheme() {
        let cfg = GraphConfig {
            falkordb: FalkorDbConfig {
                url: "redis://h:6379".to_string(),
            },
            ..Default::default()
        };
        assert_eq!(cfg.falkordb_connection_string(), "falkor://h:6379");
    }

    // ---- test_default and for_wasm constructors ----

    #[test]
    fn test_default_has_admin_allowed_and_mock_anthropic_auth() {
        let cfg = Config::test_default();
        assert!(cfg.auth.allow_admin);
        assert!(matches!(
            cfg.anthropic_auth,
            Some(AnthropicAuth::ApiKey(_))
        ));
        // Long maintenance interval so the bg loop is effectively disabled in unit tests.
        assert_eq!(cfg.pipeline.maintenance_interval_secs, 3600);
    }

    #[test]
    fn for_wasm_uses_in_memory_graph_and_openai_provider() {
        let cfg = Config::for_wasm("sk-x".into(), None, None);
        assert_eq!(cfg.graph.backend, GraphBackendType::Memory);
        assert_eq!(cfg.llm.provider, LlmProvider::OpenAI);
        assert_eq!(cfg.openai_api_key.as_deref(), Some("sk-x"));
        assert!(cfg.pipeline.infer_pre_context);
    }

    #[test]
    fn for_wasm_uses_supplied_models_when_present() {
        let cfg = Config::for_wasm(
            "sk-x".into(),
            Some("gpt-x".into()),
            Some("emb-x".into()),
        );
        assert_eq!(cfg.llm.openai.model, "gpt-x");
        assert_eq!(cfg.llm.openai.embedding_model.as_deref(), Some("emb-x"));
    }

    // ---- TOML parsing ----

    #[test]
    fn empty_toml_yields_defaults() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.port, 21693);
        assert_eq!(cfg.graph.backend, GraphBackendType::Memory);
    }

    #[test]
    fn toml_partial_override_only_changes_named_fields() {
        let toml_src = r#"
            port = 9000
            [graph]
            backend = "sqlite"
            name = "test-graph"
        "#;
        let cfg: Config = toml::from_str(toml_src).unwrap();
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.graph.backend, GraphBackendType::Sqlite);
        assert_eq!(cfg.graph.name, "test-graph");
        assert!(!cfg.llm.mock_llm);
        assert_eq!(cfg.rate_limit.requests_per_minute, 120);
    }

    #[test]
    fn toml_secrets_field_is_skipped_on_deserialise() {
        let cfg: Config = toml::from_str("port = 1234").unwrap();
        assert!(cfg.anthropic_auth.is_none());
    }

    #[test]
    fn toml_invalid_value_returns_error() {
        let result: std::result::Result<Config, _> =
            toml::from_str(r#"port = "not-a-number""#);
        assert!(result.is_err());
    }

    #[test]
    fn graph_backend_type_parses_each_known_value() {
        for (s, expected) in [
            ("falkordb", GraphBackendType::FalkorDB),
            ("memory", GraphBackendType::Memory),
            ("postgres", GraphBackendType::Postgres),
            ("qdrant", GraphBackendType::Qdrant),
            ("sqlite", GraphBackendType::Sqlite),
        ] {
            let toml_src = format!("[graph]\nbackend = \"{}\"", s);
            let cfg: Config = toml::from_str(&toml_src).unwrap();
            assert_eq!(cfg.graph.backend, expected, "for {}", s);
        }
    }

    // ---- AnthropicAuth secret hygiene ----

    #[test]
    fn anthropic_auth_debug_does_not_leak_secret() {
        let dbg = format!("{:?}", AnthropicAuth::ApiKey("sk-secret-123".into()));
        assert!(!dbg.contains("sk-secret-123"));
        assert!(dbg.contains("****"));

        let dbg2 = format!("{:?}", AnthropicAuth::OAuthToken("oauth-secret".into()));
        assert!(!dbg2.contains("oauth-secret"));
        assert!(dbg2.contains("****"));
    }

    #[test]
    fn anthropic_auth_clone_preserves_variant_and_value() {
        let original = AnthropicAuth::ApiKey("k".into());
        let cloned = original.clone();
        match cloned {
            AnthropicAuth::ApiKey(v) => assert_eq!(v, "k"),
            _ => panic!("variant changed"),
        }
    }
}
