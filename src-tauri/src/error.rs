//! Centralised error type for the SundayStudio backend.
//!
//! Tauri commands return `Result<T, AppError>` — `AppError` implements
//! `serde::Serialize` so it crosses the IPC boundary as a stable JSON shape
//! (`{ code, message }`) that the renderer can pattern-match on.
//!
//! Phase 0.1 adds the first domain variant — `Audio` — for cpal/hound failures.
//! Later phases add `Database`, `Decode`, `Export`, etc. Keep `code()` and the
//! TS `AppError` union in `src/lib/bindings/index.ts` in sync when you add one.

use serde::{Serialize, Serializer};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    /// Entity not found by ID — distinct from a general error so the renderer
    /// can render a "404" UI.
    #[error("not found: {entity} id={id}")]
    NotFound { entity: &'static str, id: String },

    /// Caller passed input that fails our domain rules.
    #[error("validation: {0}")]
    Validation(String),

    /// Audio subsystem failure (device enumeration, stream open, encode).
    #[error("audio error: {0}")]
    Audio(String),

    /// Project storage / SQLite failure.
    #[error("database: {0}")]
    Database(String),

    /// File-system / IO failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialisation/deserialisation issue.
    #[error("invalid json: {0}")]
    Json(#[from] serde_json::Error),

    /// Anything else we couldn't classify.
    #[error("internal: {0}")]
    Internal(String),
}

impl AppError {
    /// Short, machine-readable category for the renderer to switch on.
    pub fn code(&self) -> &'static str {
        match self {
            AppError::NotFound { .. } => "not_found",
            AppError::Validation(_) => "validation",
            AppError::Audio(_) => "audio",
            AppError::Database(_) => "database",
            AppError::Io(_) => "io",
            AppError::Json(_) => "json",
            AppError::Internal(_) => "internal",
        }
    }
}

/// Custom serializer so the JSON sent to the renderer has both a stable
/// `code` field (for switch statements) and the human-readable `message`.
impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        AppError::Database(e.to_string())
    }
}

impl From<sqlx::migrate::MigrateError> for AppError {
    fn from(e: sqlx::migrate::MigrateError) -> Self {
        AppError::Database(format!("migration: {e}"))
    }
}

/// Convenience alias for the project.
pub type AppResult<T> = Result<T, AppError>;
