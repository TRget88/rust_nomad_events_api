//! Custom request extractors.
//!
//! Wraps axum's built-in extractors to translate their `Rejection` types
//! into our `AppError` shape so error responses come out in the structured
//! `{"error": "<message>"}` JSON envelope every other handler uses. Without
//! this, axum's default rejection responses leak through as a separate
//! `{"message": "..."}` shape, which is confusing to API consumers.

use crate::errors::AppError;
use axum::{
    Json,
    extract::{FromRequest, Request, rejection::JsonRejection},
};
use serde::de::DeserializeOwned;

/// JSON-body extractor with friendly error messages.
///
/// Drop-in replacement for `axum::Json<T>` at the handler signature:
///
/// ```ignore
/// pub async fn create(
///     ApiJson(payload): ApiJson<CreatePayload>,
/// ) -> Result<impl IntoResponse, AppError> { ... }
/// ```
///
/// Maps axum's `JsonRejection` variants to specific `AppError::BadRequest`
/// messages instead of axum's default generic "Failed to parse the request
/// body as JSON" / "EOF while parsing JSON" strings. Empty bodies — the
/// most common malformed-request case in practice — get a clear "Did you
/// forget to send a body?" hint.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ApiJson(value)),
            Err(rejection) => Err(json_rejection_to_app_error(rejection)),
        }
    }
}

/// Convert axum's `JsonRejection` variants into our `AppError::BadRequest`
/// with messages tuned to what the caller most likely did wrong. Extracted
/// as a pub(crate) free function so tests can exercise the mapping without
/// constructing a full Request.
pub(crate) fn json_rejection_to_app_error(rejection: JsonRejection) -> AppError {
    match rejection {
        // `JsonDataError` is the "shape doesn't match T" case — body parsed
        // as JSON but didn't fit the target struct (wrong types, missing
        // required fields, etc.). The inner message names the field.
        JsonRejection::JsonDataError(e) => {
            AppError::BadRequest(format!("Invalid JSON data: {}", e))
        }
        // `JsonSyntaxError` covers both syntactically-invalid JSON AND
        // empty bodies (which fail with "EOF while parsing"). The empty-
        // body case is by far the most common — `curl -X POST` without
        // `-d` lands here — so we lead with that hint.
        JsonRejection::JsonSyntaxError(e) => AppError::BadRequest(format!(
            "Request body must be valid JSON. Did you forget to send a body? \
             (Underlying error: {})",
            e
        )),
        JsonRejection::MissingJsonContentType(_) => {
            AppError::BadRequest("Request must have Content-Type: application/json".to_string())
        }
        JsonRejection::BytesRejection(_) => {
            AppError::BadRequest("Failed to read request body".to_string())
        }
        // axum's JsonRejection is `#[non_exhaustive]`; if a future axum
        // release adds a variant we don't know about, fall back to a
        // generic BadRequest so the handler doesn't fail to compile.
        _ => AppError::BadRequest("Invalid request body".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use serde::Deserialize;
    use tower::util::ServiceExt;

    #[derive(Deserialize)]
    struct Payload {
        #[allow(dead_code)]
        name: String,
    }

    async fn echo_handler(
        ApiJson(payload): ApiJson<Payload>,
    ) -> Result<axum::response::Response, AppError> {
        Ok((StatusCode::OK, payload.name).into_response())
    }

    fn build_test_app() -> Router {
        Router::new().route("/test", post(echo_handler))
    }

    #[tokio::test]
    async fn accepts_valid_json() {
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"name":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rejects_empty_body_with_friendly_message() {
        // The "main" case: `curl -X POST` with no `-d`. Pre-refactor users
        // got axum's "Failed to parse the request body as JSON: EOF while
        // parsing a value at line 1 column 0". Now they get a clear hint
        // pointing at the missing body.
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .header("Content-Type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        // The message includes our hint about the missing body.
        assert!(
            text.contains("forget to send a body"),
            "Expected helpful empty-body message, got: {}",
            text,
        );
    }

    #[tokio::test]
    async fn rejects_malformed_json() {
        // Not empty, but not valid JSON. Pre-refactor: axum's default
        // syntax-error response. Now: our structured BadRequest.
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .header("Content-Type", "application/json")
                    .body(Body::from("not valid json {{{"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_wrong_shape_with_data_error_message() {
        // Valid JSON, wrong shape (missing `name`). Maps to `JsonDataError`
        // and gets the "Invalid JSON data: ..." prefix.
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"wrong":"field"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("Invalid JSON data"),
            "Expected 'Invalid JSON data' prefix, got: {}",
            text,
        );
    }

    #[tokio::test]
    async fn rejects_missing_content_type() {
        // `curl -X POST -d '{...}'` without `-H 'Content-Type: ...'`.
        // axum's Json rejects with MissingJsonContentType; we map to a
        // BadRequest with a clear hint.
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .body(Body::from(r#"{"name":"hello"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            text.contains("Content-Type: application/json"),
            "Expected content-type hint, got: {}",
            text,
        );
    }

    #[tokio::test]
    async fn response_uses_structured_error_envelope() {
        // Every other handler returns errors as `{"error": "<msg>"}` —
        // the extractor's rejections should match. Pre-refactor, axum's
        // built-in rejection used `{"message": ...}` — inconsistent.
        let app = build_test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/test")
                    .method("POST")
                    .header("Content-Type", "application/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Pin the envelope shape: { "error": "..." }
        assert!(
            parsed.get("error").is_some(),
            "Expected 'error' key, got: {:?}",
            parsed,
        );
    }
}
