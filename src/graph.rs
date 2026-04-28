//! Graph backend registry — backend-agnostic dispatcher.
//!
//! Concrete backends (FalkorDB, in-memory, SQLite, Postgres, Qdrant) live in
//! `crate::backends`. `GraphRegistry` selects one at startup and lazily
//! materialises per-graph clients on demand.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use tokio::sync::Mutex;

use crate::graph_backend::GraphBackend;

#[cfg(not(target_arch = "wasm32"))]
use crate::error::GraphConnectError;
#[cfg(not(target_arch = "wasm32"))]
use falkordb::{FalkorClientBuilder, FalkorConnectionInfo};

/// Backend re-export so existing callers using `crate::graph::GraphClient`
/// keep working.
#[cfg(not(target_arch = "wasm32"))]
pub use crate::backends::falkor::GraphClient;

/// Builds a per-graph backend handle. Async so backends that need network or
/// file I/O during construction (Postgres pool, Qdrant client) can `.await`
/// natively instead of `block_on`-ing on a tokio worker thread.
type GraphFactory =
    Box<dyn Fn(&str) -> BoxFuture<'static, Result<Arc<dyn GraphBackend>>> + Send + Sync>;

pub struct GraphRegistry {
    factory: GraphFactory,
    default_graph: String,
    graphs: Mutex<HashMap<String, Arc<dyn GraphBackend>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl GraphRegistry {
    /// Connect to a FalkorDB instance and return a registry that materialises
    /// per-graph clients off a pooled connection.
    pub async fn connect(connection_string: &str, default_graph: &str) -> Result<Self> {
        let info: FalkorConnectionInfo = connection_string.try_into().map_err(|e| {
            GraphConnectError::new(format!("invalid FalkorDB connection string: {e}"))
        })?;
        let client = Arc::new(
            FalkorClientBuilder::new_async()
                .with_connection_info(info)
                .build()
                .await
                .map_err(|e| {
                    GraphConnectError::new(format!("failed to connect to FalkorDB: {e}"))
                })?,
        );

        let registry = Self {
            factory: Box::new(move |name: &str| {
                let client = Arc::clone(&client);
                let name = name.to_string();
                Box::pin(async move {
                    Ok(Arc::new(GraphClient::from_client(&client, &name)) as Arc<dyn GraphBackend>)
                })
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };

        registry.get(default_graph).await;
        Ok(registry)
    }

    /// Create a registry backed by SQLite databases on disk.
    /// Each graph gets its own file: `{base_dir}/{graph_name}.db`.
    pub fn sqlite(default_graph: &str, base_path: String) -> Self {
        Self {
            factory: Box::new(move |name: &str| {
                let base_path = base_path.clone();
                let name = name.to_string();
                Box::pin(async move {
                    let path = if base_path.is_empty() {
                        std::path::PathBuf::from(format!("{name}.db"))
                    } else {
                        std::path::PathBuf::from(&base_path)
                    };
                    let graph = crate::backends::SqliteGraph::open(&name, &path)?;
                    Ok(Arc::new(graph) as Arc<dyn GraphBackend>)
                })
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        }
    }

    /// Create a registry backed by PostgreSQL.
    /// All graphs share the same Postgres instance, distinguished by the
    /// `graph_name` column.
    pub async fn postgres(connection_string: &str, default_graph: &str) -> Result<Self> {
        // Verify connectivity once before installing the factory.
        let test = crate::backends::PostgresGraph::new(connection_string, default_graph).await?;
        test.ping().await?;

        let conn_str = connection_string.to_string();
        let registry = Self {
            factory: Box::new(move |name: &str| {
                let conn_str = conn_str.clone();
                let name = name.to_string();
                Box::pin(async move {
                    let pool = crate::backends::PostgresGraph::new(&conn_str, &name).await?;
                    Ok(Arc::new(pool) as Arc<dyn GraphBackend>)
                })
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };

        registry.get(default_graph).await;
        Ok(registry)
    }

    /// Create a registry backed by Qdrant.
    pub async fn qdrant(url: &str, default_graph: &str) -> Result<Self> {
        let test = crate::backends::QdrantGraph::new(url, default_graph).await?;
        test.ping().await?;

        let url_owned = url.to_string();
        let registry = Self {
            factory: Box::new(move |name: &str| {
                let url = url_owned.clone();
                let name = name.to_string();
                Box::pin(async move {
                    let graph = crate::backends::QdrantGraph::new(&url, &name).await?;
                    Ok(Arc::new(graph) as Arc<dyn GraphBackend>)
                })
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        };

        registry.get(default_graph).await;
        Ok(registry)
    }
}

impl GraphRegistry {
    /// Create a registry backed by in-memory graphs (no database needed).
    pub fn in_memory(default_graph: &str) -> Self {
        Self {
            factory: Box::new(|name: &str| {
                let name = name.to_string();
                Box::pin(async move {
                    Ok(Arc::new(crate::backends::InMemoryGraph::new(&name)) as Arc<dyn GraphBackend>)
                })
            }),
            default_graph: default_graph.to_string(),
            graphs: Mutex::new(HashMap::new()),
        }
    }

    pub fn default_graph_name(&self) -> &str {
        &self.default_graph
    }

    pub async fn get(&self, graph_name: &str) -> Arc<dyn GraphBackend> {
        // Fast path: cached. Drop the lock before awaiting the factory so the
        // factory's I/O does not serialise other reads of the cache.
        {
            let cache = self.graphs.lock().await;
            if let Some(existing) = cache.get(graph_name) {
                tracing::debug!(graph = %graph_name, "graph_registry: cache hit");
                return Arc::clone(existing);
            }
        }

        tracing::info!(graph = %graph_name, "graph_registry: creating new graph backend");
        let arc = match (self.factory)(graph_name).await {
            Ok(a) => a,
            Err(e) => {
                // Match the prior `expect` semantics: factory errors are
                // construction-time and indicate misconfiguration.
                panic!("failed to create graph backend '{graph_name}': {e:#}");
            }
        };
        if let Err(e) = arc.setup_schema().await {
            tracing::warn!("Failed to setup schema for graph '{graph_name}': {e}");
        }

        // Re-acquire the lock and insert. Another caller may have raced us;
        // if so, return the entry they cached and drop ours.
        let mut cache = self.graphs.lock().await;
        if let Some(existing) = cache.get(graph_name) {
            return Arc::clone(existing);
        }
        cache.insert(graph_name.to_string(), Arc::clone(&arc));
        arc
    }

    pub async fn get_default(&self) -> Arc<dyn GraphBackend> {
        self.get(&self.default_graph).await
    }

    pub async fn resolve(&self, graph_name: Option<&str>) -> Arc<dyn GraphBackend> {
        match graph_name {
            Some(name) => self.get(name).await,
            None => self.get_default().await,
        }
    }

    pub async fn list(&self) -> Vec<String> {
        let cache = self.graphs.lock().await;
        cache.keys().cloned().collect()
    }

    pub async fn drop_graph(&self, graph_name: &str) -> Result<()> {
        let graph = {
            let mut cache = self.graphs.lock().await;
            cache.remove(graph_name)
        };
        match graph {
            Some(g) => g.drop_and_reinitialise().await,
            None => {
                // Not cached — create a temporary client to drop it.
                let g = self.get(graph_name).await;
                let result = g.drop_and_reinitialise().await;
                self.graphs.lock().await.remove(graph_name);
                result
            }
        }
    }
}
