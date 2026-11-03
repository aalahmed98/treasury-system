use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use domain::ErrorBody;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("too many attempts; try again in {retry_after} seconds")]
    TooManyRequests { retry_after: u64 },
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e {
            match db.code().as_deref() {
                // insufficient_privilege: an INSERT/UPDATE rejected by a row-level security policy
                Some("42501") => return ApiError::Forbidden("not entitled to that desk".into()),
                Some("23505") => return ApiError::BadRequest("duplicate record".into()),
                // Our own RAISE EXCEPTION messages are written for users; constraint names are not.
                Some("P0001") => return ApiError::BadRequest(db.message().to_string()),
                // no_data_found: a definer function could not find (or show) the record
                Some("P0002") => return ApiError::NotFound,
                Some("23514") | Some("23502") | Some("23503") => {
                    return ApiError::BadRequest("the request violates a data constraint".into());
                }
                _ => {}
            }
        }
        if matches!(e, sqlx::Error::RowNotFound) {
            return ApiError::NotFound;
        }
        ApiError::Internal(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::TooManyRequests { .. } => (StatusCode::TOO_MANY_REQUESTS, self.to_string()),
            ApiError::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        let mut res = (status, Json(ErrorBody { error: msg })).into_response();
        if let ApiError::TooManyRequests { retry_after } = self {
            res.headers_mut().insert(header::RETRY_AFTER, retry_after.into());
        }
        res
    }
}
