use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, Response};

use crate::error::Error;
use crate::models::*;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_MAX_RETRIES: u32 = 3;
const RETRYABLE_CODES: &[u16] = &[429, 502, 503, 504];

/// Async client for the Hippo knowledge graph API.
pub struct HippoClient {
    http: Client,
    base_url: String,
    max_retries: u32,
}

impl HippoClient {
    /// Create a new client.
    ///
    /// Falls back to `HIPPO_URL` / `HIPPO_API_KEY` environment variables when
    /// `base_url` / `api_key` are `None`.
    pub fn new(
        base_url: Option<&str>,
        api_key: Option<&str>,
        timeout: Option<Duration>,
        max_retries: Option<u32>,
    ) -> Result<Self, Error> {
        let base_url = base_url
            .map(String::from)
            .or_else(|| std::env::var("HIPPO_URL").ok())
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();

        let api_key = api_key
            .map(String::from)
            .or_else(|| std::env::var("HIPPO_API_KEY").ok());

        let mut headers = HeaderMap::new();
        if let Some(key) = &api_key {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {key}"))
                    .map_err(|e| Error::Decode(e.to_string()))?,
            );
        }

        let http = Client::builder()
            .default_headers(headers)
            .timeout(timeout.unwrap_or(DEFAULT_TIMEOUT))
            .build()?;

        Ok(Self {
            http,
            base_url,
            max_retries: max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
        })
    }

    // -- internal helpers -----------------------------------------------------

    fn url(&self, path: &str) -> String {
        format!("{}/api{path}", self.base_url)
    }

    async fn raise_for_status(resp: Response) -> Result<Response, Error> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }

        let code = status.as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs);

        let body = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ErrorResponse>(&body)
            .map(|e| e.error)
            .unwrap_or(body);

        match code {
            401 => Err(Error::Authentication {
                message,
                status: code,
            }),
            403 => Err(Error::Forbidden {
                message,
                status: code,
            }),
            429 => Err(Error::RateLimit {
                message,
                status: code,
                retry_after,
            }),
            _ => Err(Error::Api {
                message,
                status: code,
            }),
        }
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Result<Response, Error> {
        let url = self.url(path);
        let attempts = self.max_retries + 1;

        let mut last_err: Error = Error::Api {
            message: "no attempts made".into(),
            status: 0,
        };

        for attempt in 0..attempts {
            let mut req = self.http.request(method.clone(), &url);
            if let Some(b) = body {
                req = req.json(b);
            }

            match req.send().await {
                Ok(resp) => {
                    let code = resp.status().as_u16();
                    if RETRYABLE_CODES.contains(&code) && attempt < attempts - 1 {
                        let delay = backoff_delay(attempt);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Self::raise_for_status(resp).await;
                }
                Err(e) if (e.is_timeout() || e.is_connect()) && attempt < attempts - 1 => {
                    last_err = e.into();
                    let delay = backoff_delay(attempt);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Err(last_err)
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, Error> {
        let resp = self
            .request(reqwest::Method::POST, path, Some(body))
            .await?;
        resp.json::<T>()
            .await
            .map_err(|e| Error::Decode(e.to_string()))
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let resp = self
            .request(reqwest::Method::GET, path, None::<&()>)
            .await?;
        resp.json::<T>()
            .await
            .map_err(|e| Error::Decode(e.to_string()))
    }

    async fn delete(&self, path: &str) -> Result<(), Error> {
        self.request(reqwest::Method::DELETE, path, None::<&()>)
            .await?;
        Ok(())
    }

    // -- core endpoints -------------------------------------------------------

    pub async fn remember(
        &self,
        statement: &str,
        source_agent: Option<&str>,
        graph: Option<&str>,
        ttl_secs: Option<u64>,
    ) -> Result<RememberResponse, Error> {
        let body = RememberRequest {
            statement: statement.to_string(),
            source_agent: source_agent.map(String::from),
            source_credibility_hint: None,
            graph: graph.map(String::from),
            ttl_secs,
        };
        self.post("/remember", &body).await
    }

    /// Send a fully-typed `RememberRequest`, including
    /// `source_credibility_hint`. Prefer this when you need fields beyond
    /// the convenience method [`Self::remember`].
    pub async fn remember_with(
        &self,
        request: &RememberRequest,
    ) -> Result<RememberResponse, Error> {
        self.post("/remember", request).await
    }

    pub async fn remember_batch(
        &self,
        statements: Vec<String>,
        source_agent: Option<&str>,
        parallel: bool,
        graph: Option<&str>,
        ttl_secs: Option<u64>,
    ) -> Result<BatchRememberResponse, Error> {
        let body = BatchRememberRequest {
            statements,
            source_agent: source_agent.map(String::from),
            parallel,
            graph: graph.map(String::from),
            ttl_secs,
        };
        self.post("/remember/batch", &body).await
    }

    pub async fn context(
        &self,
        query: &str,
        limit: Option<usize>,
        max_hops: Option<usize>,
        graph: Option<&str>,
    ) -> Result<ContextResponse, Error> {
        let body = ContextRequest {
            query: query.to_string(),
            limit,
            max_hops,
            memory_tier_filter: None,
            graph: graph.map(String::from),
            at: None,
            scoring: None,
        };
        self.post("/context", &body).await
    }

    /// Send a fully-typed `ContextRequest` with optional `memory_tier_filter`,
    /// `at` (temporal slice), and custom `scoring`.
    pub async fn context_with(
        &self,
        request: &ContextRequest,
    ) -> Result<ContextResponse, Error> {
        self.post("/context", request).await
    }

    pub async fn ask(
        &self,
        question: &str,
        limit: Option<usize>,
        graph: Option<&str>,
        verbose: bool,
    ) -> Result<AskResponse, Error> {
        let body = AskRequest {
            question: question.to_string(),
            limit,
            graph: graph.map(String::from),
            verbose,
            max_iterations: 1,
        };
        self.post("/ask", &body).await
    }

    /// Send a fully-typed `AskRequest`, including `max_iterations`.
    pub async fn ask_with(&self, request: &AskRequest) -> Result<AskResponse, Error> {
        self.post("/ask", request).await
    }

    // -- REST resources -------------------------------------------------------

    /// `GET /entities/{id}` — fetch a single entity. Response shape is the
    /// server-side `Entity` record; returned as `serde_json::Value` because the
    /// embedding vector and timestamps are not part of the public API crate.
    pub async fn get_entity(
        &self,
        id: &str,
        graph: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        self.get(&with_graph(&format!("/entities/{id}"), graph))
            .await
    }

    /// `DELETE /entities/{id}`. Returns the server's confirmation payload
    /// (`id`, `name`, `edges_invalidated`).
    pub async fn delete_entity(
        &self,
        id: &str,
        graph: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        let resp = self
            .request(
                reqwest::Method::DELETE,
                &with_graph(&format!("/entities/{id}"), graph),
                None::<&()>,
            )
            .await?;
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| Error::Decode(e.to_string()))
    }

    /// `GET /entities/{id}/edges`.
    pub async fn entity_edges(
        &self,
        id: &str,
        graph: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        self.get(&with_graph(&format!("/entities/{id}/edges"), graph))
            .await
    }

    /// `GET /edges/{id}`.
    pub async fn get_edge(
        &self,
        id: i64,
        graph: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        self.get(&with_graph(&format!("/edges/{id}"), graph)).await
    }

    /// `GET /edges/{id}/provenance`.
    pub async fn edge_provenance(
        &self,
        id: i64,
        graph: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        self.get(&with_graph(&format!("/edges/{id}/provenance"), graph))
            .await
    }

    // -- destructive operations ----------------------------------------------

    /// `POST /retract` — explicit user/agent retraction of a fact.
    pub async fn retract(&self, request: &RetractRequest) -> Result<RetractResponse, Error> {
        self.post("/retract", request).await
    }

    /// `POST /correct` — retract an old fact and observe a new one in one call.
    pub async fn correct(&self, request: &CorrectRequest) -> Result<CorrectResponse, Error> {
        self.post("/correct", request).await
    }

    // -- operations / observability ------------------------------------------

    /// `POST /maintain` — run a single maintenance cycle. Returns the server's
    /// `DreamReport` as untyped JSON.
    pub async fn maintain(&self) -> Result<serde_json::Value, Error> {
        self.post("/maintain", &serde_json::json!({})).await
    }

    /// `GET /graph` — dump the full graph (defaults to JSON; pass `format` for
    /// `graphml` or `csv` to get the raw export body as text).
    pub async fn graph(
        &self,
        graph: Option<&str>,
    ) -> Result<serde_json::Value, Error> {
        self.get(&with_graph("/graph", graph)).await
    }

    /// `GET /graph?format=…` returning the raw export body.
    pub async fn graph_export(
        &self,
        graph: Option<&str>,
        format: &str,
    ) -> Result<String, Error> {
        let mut path = format!("/graph?format={format}");
        if let Some(g) = graph {
            path.push_str(&format!("&graph={g}"));
        }
        let resp = self
            .request(reqwest::Method::GET, &path, None::<&()>)
            .await?;
        resp.text().await.map_err(|e| Error::Decode(e.to_string()))
    }

    /// `GET /metrics` — Prometheus-format metrics as raw text.
    pub async fn metrics(&self) -> Result<String, Error> {
        let resp = self
            .request(reqwest::Method::GET, "/metrics", None::<&()>)
            .await?;
        resp.text().await.map_err(|e| Error::Decode(e.to_string()))
    }

    /// `GET /openapi.yaml` as raw text.
    pub async fn openapi(&self) -> Result<String, Error> {
        let resp = self
            .request(reqwest::Method::GET, "/openapi.yaml", None::<&()>)
            .await?;
        resp.text().await.map_err(|e| Error::Decode(e.to_string()))
    }

    // -- graphs ---------------------------------------------------------------

    /// `GET /graphs` — list known graphs.
    pub async fn list_graphs(&self) -> Result<GraphsListResponse, Error> {
        self.get("/graphs").await
    }

    /// `DELETE /graphs/drop/{name}` — drop a graph (admin only).
    pub async fn drop_graph(&self, name: &str) -> Result<serde_json::Value, Error> {
        let resp = self
            .request(
                reqwest::Method::DELETE,
                &format!("/graphs/drop/{name}"),
                None::<&()>,
            )
            .await?;
        resp.json::<serde_json::Value>()
            .await
            .map_err(|e| Error::Decode(e.to_string()))
    }

    /// `POST /seed` — admin direct seeding of entities/edges.
    pub async fn seed(
        &self,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        self.post("/seed", body).await
    }

    /// `POST /admin/backup` — returns the backup payload as raw JSON text.
    pub async fn backup(&self, graph: Option<&str>) -> Result<String, Error> {
        let body = serde_json::json!({ "graph": graph });
        let resp = self
            .request(reqwest::Method::POST, "/admin/backup", Some(&body))
            .await?;
        resp.text().await.map_err(|e| Error::Decode(e.to_string()))
    }

    /// `POST /admin/restore` — restore from a backup payload.
    pub async fn restore(
        &self,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        self.post("/admin/restore", body).await
    }

    /// `GET /admin/audit` — admin audit log entries, newest first.
    pub async fn audit(
        &self,
        user_id: Option<&str>,
        action: Option<&str>,
        limit: Option<usize>,
    ) -> Result<AuditResponse, Error> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(u) = user_id {
            params.push(("user_id", u.to_string()));
        }
        if let Some(a) = action {
            params.push(("action", a.to_string()));
        }
        if let Some(l) = limit {
            params.push(("limit", l.to_string()));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = params
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            format!("?{}", pairs.join("&"))
        };
        self.get(&format!("/admin/audit{qs}")).await
    }

    // -- admin endpoints ------------------------------------------------------

    pub async fn create_user(
        &self,
        user_id: &str,
        display_name: &str,
        role: Option<&str>,
        graphs: Option<Vec<String>>,
    ) -> Result<CreateUserResponse, Error> {
        let body = CreateUserRequest {
            user_id: user_id.to_string(),
            display_name: display_name.to_string(),
            role: role.map(String::from),
            graphs,
        };
        self.post("/admin/users", &body).await
    }

    pub async fn list_users(&self) -> Result<ListUsersResponse, Error> {
        self.get("/admin/users").await
    }

    pub async fn delete_user(&self, user_id: &str) -> Result<(), Error> {
        self.delete(&format!("/admin/users/{user_id}")).await
    }

    pub async fn create_key(
        &self,
        user_id: &str,
        label: &str,
    ) -> Result<CreateKeyResponse, Error> {
        let body = CreateKeyRequest {
            label: label.to_string(),
        };
        self.post(&format!("/admin/users/{user_id}/keys"), &body)
            .await
    }

    pub async fn list_keys(&self, user_id: &str) -> Result<ListKeysResponse, Error> {
        self.get(&format!("/admin/users/{user_id}/keys")).await
    }

    pub async fn delete_key(&self, user_id: &str, label: &str) -> Result<(), Error> {
        self.delete(&format!("/admin/users/{user_id}/keys/{label}"))
            .await
    }

    // -- observability --------------------------------------------------------

    pub async fn health(&self) -> Result<HealthResponse, Error> {
        self.get("/health").await
    }
}

fn with_graph(path: &str, graph: Option<&str>) -> String {
    match graph {
        Some(g) => format!("{path}?graph={g}"),
        None => path.to_string(),
    }
}

fn backoff_delay(attempt: u32) -> Duration {
    let base_ms = 500u64 * 2u64.pow(attempt);
    let jitter = rand::random::<u64>() % (base_ms / 2 + 1);
    Duration::from_millis(base_ms + jitter)
}
