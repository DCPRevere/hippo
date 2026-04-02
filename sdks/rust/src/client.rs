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
            429 => {
                // TODO: parse Retry-After header
                Err(Error::RateLimit {
                    message,
                    status: code,
                    retry_after: None,
                })
            }
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

        let mut last_err: Option<Error> = None;

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
                    last_err = Some(e.into());
                    let delay = backoff_delay(attempt);
                    tokio::time::sleep(delay).await;
                }
                Err(e) => return Err(e.into()),
            }
        }

        Err(last_err.unwrap())
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

fn backoff_delay(attempt: u32) -> Duration {
    let base_ms = 500u64 * 2u64.pow(attempt);
    let jitter = rand::random::<u64>() % (base_ms / 2 + 1);
    Duration::from_millis(base_ms + jitter)
}
