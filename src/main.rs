use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use hippo::config::{self, Config};
use hippo::graph::GraphRegistry;
use hippo::llm::{self, LlmClient};
use hippo::state::AppState;
use hippo::{credibility, http, pipeline};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hippo=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!("Starting hippo on port {}", config.port);

    // Connect to graph backend
    let graphs = if config.graph_backend == config::GraphBackendType::Memory {
        tracing::info!("Using in-memory graph backend");
        GraphRegistry::in_memory(&config.graph_name)
    } else {
        let connection_string = config.falkordb_connection_string();
        tracing::info!("Connecting to FalkorDB at {connection_string}");
        let registry = GraphRegistry::connect(&connection_string, &config.graph_name).await?;
        registry.get_default().await.setup_schema().await?;
        registry
    };

    // Build LLM client
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let auth = match &config.anthropic_auth {
        config::AnthropicAuth::ApiKey(k) => llm::AnthropicAuth::ApiKey(k.clone()),
        config::AnthropicAuth::OAuthToken(t) => llm::AnthropicAuth::OAuthToken(t.clone()),
    };
    let fixture_mode = match config.fixture_mode.as_str() {
        "record" => llm::FixtureMode::Record,
        "replay" => llm::FixtureMode::Replay,
        _ => llm::FixtureMode::None,
    };
    let mut llm = LlmClient::new(
        auth,
        config.anthropic_model.clone(),
        config.ollama_url.clone(),
        http_client,
        fixture_mode,
        std::path::PathBuf::from(&config.fixture_path),
        config.mock_llm,
        config.llm_max_tokens,
    );

    if config.llm_provider == config::LlmProvider::OpenAI {
        tracing::info!("Using OpenAI provider (model: {})", config.openai_model);
        llm = llm.with_openai(
            config.openai_api_key.clone(),
            config.openai_base_url.clone(),
            config.openai_model.clone(),
            config.openai_embedding_model.clone(),
        );
    }

    // Hydrate credibility registry from FalkorDB
    let mut cred_registry = credibility::CredibilityRegistry::new();
    match graphs.get_default().await.load_all_source_credibility().await {
        Ok(entries) => {
            let count: usize = entries.len();
            cred_registry.hydrate(entries);
            tracing::info!("Loaded {count} source credibility entries from FalkorDB");
        }
        Err(e) => {
            tracing::warn!("Failed to load credibility from FalkorDB, starting empty: {e}");
        }
    }

    let (recent_nodes_tx, recent_nodes_rx) = tokio::sync::mpsc::channel::<String>(200);

    let state = Arc::new(AppState {
        graphs: Some(graphs),
        llm: Arc::new(llm),
        config,
        recent_nodes_tx,
        recent_nodes_rx: Arc::new(Mutex::new(recent_nodes_rx)),
        recent_node_ids: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        checked_pairs: Arc::new(tokio::sync::RwLock::new(HashSet::new())),
        metrics: Arc::new(hippo::state::MetricsState::new()),
        credibility: Arc::new(tokio::sync::RwLock::new(cred_registry)),
    });

    // Spawn background maintenance loop
    let maintenance_state = Arc::clone(&state);
    tokio::spawn(pipeline::maintain::run_maintenance_loop(maintenance_state));

    // Start HTTP server
    let addr = format!("0.0.0.0:{}", state.config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Listening on {addr}");

    axum::serve(listener, http::router(state)).await?;
    Ok(())
}
