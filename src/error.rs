#[cfg(not(target_arch = "wasm32"))]
use axum::http::StatusCode;
#[cfg(not(target_arch = "wasm32"))]
use axum::response::IntoResponse;

#[cfg(target_arch = "wasm32")]
mod status_compat {
    #[derive(Debug, Clone, Copy)]
    pub struct StatusCode(u16);
    impl StatusCode {
        pub const BAD_REQUEST: Self = Self(400);
        pub const UNAUTHORIZED: Self = Self(401);
        pub const FORBIDDEN: Self = Self(403);
        pub const NOT_FOUND: Self = Self(404);
        pub const TOO_MANY_REQUESTS: Self = Self(429);
        pub const INTERNAL_SERVER_ERROR: Self = Self(500);
        pub const BAD_GATEWAY: Self = Self(502);
        pub const SERVICE_UNAVAILABLE: Self = Self(503);
        pub const GATEWAY_TIMEOUT: Self = Self(504);
        pub fn as_u16(&self) -> u16 {
            self.0
        }
    }
}
#[cfg(target_arch = "wasm32")]
use status_compat::StatusCode;

#[cfg(not(target_arch = "wasm32"))]
use crate::models::ErrorResponse;

/// Application-level error that carries an HTTP status code.
///
/// Pipeline code can return `AppError` directly (via the `From<anyhow::Error>`
/// impl which defaults to 500), or use the constructors for specific status
/// codes when the error category is known.
#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.status.as_u16(), self.message)
    }
}

impl std::error::Error for AppError {}

#[cfg(not(target_arch = "wasm32"))]
impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let body = serde_json::to_string_pretty(&ErrorResponse {
            error: self.message,
        })
        .unwrap_or_default();

        let mut resp = body.into_response();
        *resp.status_mut() = self.status;
        resp.headers_mut().insert(
            "content-type",
            axum::http::HeaderValue::from_static("application/json"),
        );
        resp
    }
}

/// Converts an `anyhow::Error` into an `AppError`, inspecting the error chain
/// for known types to select the appropriate HTTP status code.
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        // If someone already wrapped an AppError, unwrap it.
        if let Some(app_err) = err.downcast_ref::<AppError>() {
            return Self::new(app_err.status, app_err.message.clone());
        }

        let msg = err.to_string();

        // LLM upstream failures → 502
        #[cfg(not(target_arch = "wasm32"))]
        if err.downcast_ref::<crate::llm::LlmError>().is_some() {
            return Self::bad_gateway(msg);
        }

        // Request timeout / connection failure (reqwest) → 504 / 503
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
            if reqwest_err.is_timeout() {
                return Self::new(StatusCode::GATEWAY_TIMEOUT, msg);
            }
            if reqwest_err.is_connect() {
                return Self::unavailable(msg);
            }
        }

        // Graph backend connectivity → 503
        if err.downcast_ref::<GraphConnectError>().is_some() {
            return Self::unavailable(msg);
        }

        Self::internal(msg)
    }
}

/// Typed error for graph backend connectivity failures.
///
/// Used instead of bare `.context("failed to connect to ...")` strings so that
/// `AppError` can downcast to this type for 503 classification.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct GraphConnectError {
    pub message: String,
}

impl GraphConnectError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn constructors_set_expected_status_codes() {
        assert_eq!(AppError::bad_request("x").status, StatusCode::BAD_REQUEST);
        assert_eq!(AppError::not_found("x").status, StatusCode::NOT_FOUND);
        assert_eq!(AppError::bad_gateway("x").status, StatusCode::BAD_GATEWAY);
        assert_eq!(AppError::unauthorized("x").status, StatusCode::UNAUTHORIZED);
        assert_eq!(AppError::forbidden("x").status, StatusCode::FORBIDDEN);
        assert_eq!(
            AppError::too_many_requests("x").status,
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            AppError::unavailable("x").status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            AppError::internal("x").status,
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn display_format_is_status_colon_message() {
        let err = AppError::bad_request("missing field");
        assert_eq!(err.to_string(), "400: missing field");
    }

    #[test]
    fn anyhow_default_maps_to_internal() {
        let app: AppError = anyhow!("boom").into();
        assert_eq!(app.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(app.message.contains("boom"));
    }

    #[test]
    fn anyhow_unwraps_existing_app_error() {
        let original = AppError::not_found("entity 'Alice'");
        let wrapped: anyhow::Error = anyhow!(original);
        let app: AppError = wrapped.into();
        assert_eq!(app.status, StatusCode::NOT_FOUND);
        assert_eq!(app.message, "entity 'Alice'");
    }

    #[test]
    fn anyhow_with_llm_error_maps_to_bad_gateway() {
        let llm = crate::llm::LlmError::AnthropicApi {
            status: 500,
            body: "upstream blew up".into(),
        };
        let app: AppError = anyhow::Error::new(llm).into();
        assert_eq!(app.status, StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn anyhow_with_graph_connect_error_maps_to_unavailable() {
        let gc = GraphConnectError::new("falkordb unreachable");
        let app: AppError = anyhow::Error::new(gc).into();
        assert_eq!(app.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn graph_connect_error_display_contains_message() {
        let gc = GraphConnectError::new("redis://localhost: refused");
        assert!(gc.to_string().contains("redis://localhost: refused"));
    }

    #[tokio::test]
    async fn into_response_sets_status_and_json_body() {
        use axum::body::to_bytes;
        let resp = AppError::not_found("missing").into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"], "missing");
    }

    /// Regression: the body must be the JSON `ErrorResponse`, not the raw
    /// message — clients parse `.error`. If anyone replaces the serialiser,
    /// this test catches it.
    #[tokio::test]
    async fn into_response_body_is_error_response_envelope() {
        use axum::body::to_bytes;
        let resp = AppError::bad_request("no field foo").into_response();
        let bytes = to_bytes(resp.into_body(), 1024).await.unwrap();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.contains(r#""error""#));
        assert!(s.contains("no field foo"));
    }
}
