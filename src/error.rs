use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found: {0}")]
    NotFound(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("forbidden")]
    Forbidden,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("internal server error")]
    Internal(#[from] anyhow::Error),

    #[error("database error")]
    Database(#[from] sqlx::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, "forbidden".to_string()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            AppError::Validation(msg) => (StatusCode::UNPROCESSABLE_ENTITY, msg.clone()),
            AppError::Internal(e) => {
                tracing::error!("internal error: {e:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Database(e) => {
                // Unique constraint violation → 409 Conflict
                if let sqlx::Error::Database(db_err) = &e {
                    if db_err.code().as_deref() == Some("23505") {
                        return (StatusCode::CONFLICT, Json(json!({ "error": "already exists" }))).into_response();
                    }
                }
                tracing::error!("database error: {e:?}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };

        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
