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
        pub fn as_u16(&self) -> u16 { self.0 }
    }
}
#[cfg(target_arch = "wasm32")]
use status_compat::StatusCode;

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
        Self { status, message: message.into() }
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
        Self { message: message.into() }
    }
}
