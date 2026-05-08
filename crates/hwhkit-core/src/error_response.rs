//! RFC 7807 Problem Details for HTTP APIs.
//!
//! Provides a standard `ApiError` enum and `IntoResponse` impl that emits
//! `application/problem+json` payloads. Construct values directly or via
//! the helpers (`ApiError::not_found`, etc.). Custom variants can be
//! created with `ApiError::custom`.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type ApiResult<T> = std::result::Result<T, ApiError>;

const PROBLEM_JSON: &str = "application/problem+json";

/// Structured RFC 7807 problem-details payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProblemDetails {
    /// URI reference identifying the problem type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Short human-readable summary.
    pub title: String,
    /// HTTP status code.
    pub status: u16,
    /// Detailed human-readable explanation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// URI reference identifying this specific occurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Optional structured field-level errors (validation).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub errors: Vec<FieldError>,
    /// Free-form extension members.
    #[serde(flatten)]
    pub extensions: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

impl FieldError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Common error variants applications run into. Convert to a [`ProblemDetails`]
/// payload by relying on `IntoResponse`, or call [`ApiError::into_problem`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ApiError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("validation failed")]
    Validation(Vec<FieldError>),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("rate limit: {0}")]
    RateLimit(String),
    #[error("internal error: {0}")]
    Internal(String),
    #[error("custom error: {title}")]
    Custom {
        status: StatusCode,
        kind: String,
        title: String,
        detail: Option<String>,
    },
}

impl ApiError {
    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::NotFound(detail.into())
    }
    pub fn unauthorized(detail: impl Into<String>) -> Self {
        Self::Unauthorized(detail.into())
    }
    pub fn forbidden(detail: impl Into<String>) -> Self {
        Self::Forbidden(detail.into())
    }
    pub fn conflict(detail: impl Into<String>) -> Self {
        Self::Conflict(detail.into())
    }
    pub fn rate_limit(detail: impl Into<String>) -> Self {
        Self::RateLimit(detail.into())
    }
    pub fn internal(detail: impl Into<String>) -> Self {
        Self::Internal(detail.into())
    }
    pub fn validation(errors: Vec<FieldError>) -> Self {
        Self::Validation(errors)
    }
    pub fn custom(
        status: StatusCode,
        kind: impl Into<String>,
        title: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self::Custom {
            status,
            kind: kind.into(),
            title: title.into(),
            detail,
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::RateLimit(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Custom { status, .. } => *status,
        }
    }

    pub fn into_problem(self) -> ProblemDetails {
        let status = self.status();
        match self {
            Self::NotFound(detail) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Not Found".to_string(),
                status: status.as_u16(),
                detail: Some(detail),
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
            Self::Unauthorized(detail) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Unauthorized".to_string(),
                status: status.as_u16(),
                detail: Some(detail),
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
            Self::Forbidden(detail) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Forbidden".to_string(),
                status: status.as_u16(),
                detail: Some(detail),
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
            Self::Validation(errors) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Validation Failed".to_string(),
                status: status.as_u16(),
                detail: Some("one or more fields failed validation".to_string()),
                instance: None,
                errors,
                extensions: Default::default(),
            },
            Self::Conflict(detail) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Conflict".to_string(),
                status: status.as_u16(),
                detail: Some(detail),
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
            Self::RateLimit(detail) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Too Many Requests".to_string(),
                status: status.as_u16(),
                detail: Some(detail),
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
            Self::Internal(detail) => ProblemDetails {
                kind: "about:blank".to_string(),
                title: "Internal Server Error".to_string(),
                status: status.as_u16(),
                detail: Some(detail),
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
            Self::Custom {
                status,
                kind,
                title,
                detail,
            } => ProblemDetails {
                kind,
                title,
                status: status.as_u16(),
                detail,
                instance: None,
                errors: Vec::new(),
                extensions: Default::default(),
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let problem = self.into_problem();
        let status =
            StatusCode::from_u16(problem.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&problem).unwrap_or_else(|_| b"{}".to_vec());
        let mut response = (status, body).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_status_matches_variant() {
        assert_eq!(ApiError::not_found("x").status(), StatusCode::NOT_FOUND);
        assert_eq!(
            ApiError::unauthorized("x").status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ApiError::validation(vec![FieldError {
                field: "name".into(),
                message: "required".into()
            }])
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn into_problem_includes_field_errors() {
        let err = ApiError::validation(vec![FieldError {
            field: "name".into(),
            message: "required".into(),
        }]);
        let p = err.into_problem();
        assert_eq!(p.errors.len(), 1);
        assert_eq!(p.errors[0].field, "name");
    }
}
