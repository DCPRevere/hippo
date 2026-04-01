use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

use crate::config::HippoConfig;
use crate::models::*;

/// HTTP client for the Hippo REST API.
///
/// Cloneable (shares the underlying connection pool).
#[derive(Debug, Clone)]
pub struct HippoClient {
    http: reqwest::Client,
    base_url: String,
    graph: Option<String>,
}

impl HippoClient {
    /// Create a new client from the given configuration.
    pub fn new(config: &HippoConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(ref key) = config.api_key {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {key}"))
                    .context("invalid API key characters")?,
            );
        }

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            graph: config.graph.clone(),
        })
    }

    /// Store a statement in Hippo.
    pub async fn remember(&self, statement: &str, source_agent: &str) -> Result<RememberResponse> {
        let body = RememberRequest {
            statement: statement.to_string(),
            source_agent: Some(source_agent.to_string()),
            graph: self.graph.clone(),
        };

        let resp = self
            .http
            .post(format!("{}/remember", self.base_url))
            .json(&body)
            .send()
            .await
            .context("hippo /remember request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("hippo /remember returned {status}: {text}");
        }

        resp.json::<RememberResponse>()
            .await
            .context("failed to parse /remember response")
    }

    /// Query Hippo for relevant context.
    pub async fn context(&self, query: &str, limit: Option<usize>) -> Result<ContextResponse> {
        let body = ContextRequest {
            query: query.to_string(),
            limit,
            graph: self.graph.clone(),
        };

        let resp = self
            .http
            .post(format!("{}/context", self.base_url))
            .json(&body)
            .send()
            .await
            .context("hippo /context request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("hippo /context returned {status}: {text}");
        }

        resp.json::<ContextResponse>()
            .await
            .context("failed to parse /context response")
    }

    /// Ask a natural-language question.
    pub async fn ask(&self, question: &str) -> Result<AskApiResponse> {
        let body = AskApiRequest {
            question: question.to_string(),
            graph: self.graph.clone(),
        };

        let resp = self
            .http
            .post(format!("{}/ask", self.base_url))
            .json(&body)
            .send()
            .await
            .context("hippo /ask request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("hippo /ask returned {status}: {text}");
        }

        resp.json::<AskApiResponse>()
            .await
            .context("failed to parse /ask response")
    }

    /// Health check. Returns `true` when Hippo reports status "ok".
    pub async fn health(&self) -> Result<bool> {
        let resp = self
            .http
            .get(format!("{}/health", self.base_url))
            .send()
            .await
            .context("hippo /health request failed")?;

        if !resp.status().is_success() {
            return Ok(false);
        }

        let body = resp
            .json::<HealthResponse>()
            .await
            .context("failed to parse /health response")?;

        Ok(body.status == "ok")
    }

    /// Fetch graph-level statistics. Used by `Memory::count()`.
    pub async fn graph_stats(&self) -> Result<GraphStats> {
        let mut url = format!("{}/graph", self.base_url);
        if let Some(ref g) = self.graph {
            url.push_str(&format!("?graph={g}"));
        }

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("hippo /graph request failed")?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("hippo /graph returned {status}: {text}");
        }

        resp.json::<GraphStats>()
            .await
            .context("failed to parse /graph response")
    }
}
