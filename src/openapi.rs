use axum::response::IntoResponse;

/// The OpenAPI 3.1 specification embedded at compile time.
const SPEC: &str = include_str!("../docs/openapi.yaml");

pub async fn openapi_handler() -> impl IntoResponse {
    (
        axum::http::StatusCode::OK,
        [("content-type", "text/yaml")],
        SPEC,
    )
}
