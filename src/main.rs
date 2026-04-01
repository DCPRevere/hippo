use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

use hippo::auth::GraphUserStore;
use hippo::config::{self, AnthropicAuth, Config};
use hippo::graph::GraphRegistry;
use hippo::llm::{self, LlmClient};
use hippo::state::AppState;
use hippo::{audit, credibility, http, pipeline};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "hippo=info,tower_http=info".into()),
        )
        .init();

    let config = Config::load()?;
    tracing::info!("Starting hippo on port {}", config.port);

    // Connect to graph backend
    let graphs = match config.graph.backend {
        config::GraphBackendType::Memory => {
            tracing::info!("Using in-memory graph backend");
            GraphRegistry::in_memory(&config.graph.name)
        }
        config::GraphBackendType::Sqlite => {
            tracing::info!("Using SQLite graph backend at {}", config.graph.sqlite.path);
            GraphRegistry::sqlite(&config.graph.name, config.graph.sqlite.path.clone())
        }
        config::GraphBackendType::Postgres => {
            let url = &config.graph.postgres.url;
            tracing::info!("Using PostgreSQL graph backend at {url}");
            GraphRegistry::postgres(url, &config.graph.name).await?
        }
        config::GraphBackendType::Qdrant => {
            let url = &config.graph.qdrant.url;
            tracing::info!("Using Qdrant graph backend at {url}");
            GraphRegistry::qdrant(url, &config.graph.name).await?
        }
        config::GraphBackendType::FalkorDB => {
            let connection_string = config.graph.falkordb_connection_string();
            tracing::info!("Connecting to FalkorDB at {connection_string}");
            GraphRegistry::connect(&connection_string, &config.graph.name).await?
        }
    };

    // Build LLM client
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let auth = match config.anthropic_auth.as_ref().expect("anthropic_auth resolved in Config::load") {
        AnthropicAuth::ApiKey(k) => llm::AnthropicAuth::ApiKey(k.clone()),
        AnthropicAuth::OAuthToken(t) => llm::AnthropicAuth::OAuthToken(t.clone()),
    };
    let fixture_mode = match config.eval.fixture_mode.as_str() {
        "record" => llm::FixtureMode::Record,
        "replay" => llm::FixtureMode::Replay,
        _ => llm::FixtureMode::None,
    };
    let mut llm = LlmClient::new(
        auth,
        config.llm.anthropic.model.clone(),
        config.llm.ollama.url.clone(),
        http_client,
        fixture_mode,
        std::path::PathBuf::from(&config.eval.fixture_path),
        config.llm.mock_llm,
        config.llm.max_tokens,
        config.llm.extraction_prompt.clone(),
    );

    if config.llm.provider == config::LlmProvider::OpenAI {
        tracing::info!("Using OpenAI provider (model: {})", config.llm.openai.model);
        llm = llm.with_openai(
            config.openai_api_key.clone(),
            config.llm.openai.base_url.clone(),
            config.llm.openai.model.clone(),
            config.llm.openai.embedding_model.clone(),
        );
    }

    // Credibility registry starts empty; entries accumulate during operation.
    let cred_registry = credibility::CredibilityRegistry::new();

    // Set up auth
    if config.auth.insecure {
        tracing::warn!("INSECURE MODE: authentication is disabled. Do not use in production.");
    }

    let user_store: Option<Arc<GraphUserStore>> = if config.auth.enabled && !config.auth.insecure {
        let users_graph = graphs.get(hippo::auth::USERS_GRAPH).await;
        let store = GraphUserStore::new(users_graph).await?;

        // First-run bootstrap: create an initial admin user if none exist
        if !store.has_users().await {
            let raw_key = store
                .create_user("admin", "Admin", "admin", &["*".to_string()])
                .await?;
            eprintln!("==========================================================");
            eprintln!("  No users found. Creating initial admin user.");
            eprintln!();
            eprintln!("  API key: {raw_key}");
            eprintln!();
            eprintln!("  Save this key — it will not be shown again.");
            eprintln!("==========================================================");
        }

        tracing::info!("Auth enabled (users stored in {})", hippo::auth::USERS_GRAPH);
        Some(Arc::new(store))
    } else {
        tracing::info!("Auth disabled (set auth.enabled = true to enable)");
        None
    };

    // Initialise audit log
    let audit = {
        let audit_graph = graphs.get(audit::AUDIT_GRAPH).await;
        Some(Arc::new(audit::AuditLog::new(audit_graph)))
    };

    // Rate limiter
    let rate_limiter = if config.rate_limit.enabled {
        tracing::info!(
            "Rate limiting enabled ({} req/min per user)",
            config.rate_limit.requests_per_minute
        );
        Some(hippo::rate_limit::RateLimiter::new(config.rate_limit.requests_per_minute))
    } else {
        None
    };

    let (recent_nodes_tx, recent_nodes_rx) = tokio::sync::mpsc::channel::<String>(200);
    let (event_tx, _) = tokio::sync::broadcast::channel::<hippo::events::GraphEvent>(256);

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
        event_tx,
        user_store: user_store.map(|s| s as Arc<dyn hippo::auth::UserStore>),
        audit,
        rate_limiter,
    });

    // Shutdown signal — broadcasts to all receivers when SIGINT/SIGTERM arrives
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());

    // Spawn background maintenance loop
    let maintenance_state = Arc::clone(&state);
    tokio::spawn(pipeline::maintain::run_maintenance_loop(maintenance_state, shutdown_rx));

    // Start HTTP server
    let addr = format!("0.0.0.0:{}", state.config.port);
    let router = http::router(state.clone());

    if state.config.tls.enabled {
        let tls_config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
            &state.config.tls.cert_path,
            &state.config.tls.key_path,
        )
        .await?;
        tracing::info!("Listening on {addr} (HTTPS/TLS)");

        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown_signal(shutdown_tx).await;
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });

        axum_server::bind_rustls(addr.parse()?, tls_config)
            .handle(handle)
            .serve(router.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("Listening on {addr} (HTTP)");

        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal(shutdown_tx))
            .await?;
    }

    tracing::info!("Shutdown complete");
    Ok(())
}

async fn shutdown_signal(shutdown_tx: tokio::sync::watch::Sender<()>) {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received SIGINT, shutting down"),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down"),
    }

    // Notify the maintenance loop to stop
    drop(shutdown_tx);
}
