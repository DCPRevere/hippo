use axum::http::StatusCode;
use axum::response::IntoResponse;

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
/// for known categories to select the appropriate HTTP status code.
impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        // If someone already wrapped an AppError, unwrap it.
        if let Some(app_err) = err.downcast_ref::<AppError>() {
            return Self::new(app_err.status, app_err.message.clone());
        }

        let msg = err.to_string();
        let chain = format!("{err:#}");

        // LLM upstream failures → 502
        if chain.contains("failed to call Anthropic API")
            || chain.contains("failed to call OpenAI API")
            || chain.contains("Anthropic API error")
            || chain.contains("OpenAI API error")
        {
            return Self::bad_gateway(msg);
        }

        // Graph backend connectivity → 503
        if chain.contains("ping failed")
            || chain.contains("failed to connect to FalkorDB")
        {
            return Self::unavailable(msg);
        }

        // Request timeout (reqwest) → 504
        if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>() {
            if reqwest_err.is_timeout() {
                return Self::new(StatusCode::GATEWAY_TIMEOUT, msg);
            }
            if reqwest_err.is_connect() {
                return Self::unavailable(msg);
            }
        }

        Self::internal(msg)
    }
}
