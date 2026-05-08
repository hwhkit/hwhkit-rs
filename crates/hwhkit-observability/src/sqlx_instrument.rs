//! Lightweight OpenTelemetry instrumentation helpers for `sqlx`.
//!
//! Rather than wrap [`sqlx::Executor`] (which is impractical because the
//! trait is not dyn-compatible and bounds vary across query types), this
//! module exposes:
//!
//! - [`SqlxSpan::query`]: a constructor that opens a `tracing` span with
//!   the canonical OpenTelemetry semantic-conventions database attributes
//!   (`db.system`, `db.statement`, …) and lets the caller `.instrument()`
//!   the underlying sqlx future.
//!
//! Usage:
//!
//! ```ignore
//! use hwhkit_observability::sqlx_instrument::SqlxSpan;
//! use tracing::Instrument;
//!
//! let stmt = "SELECT name FROM users WHERE id = $1";
//! let span = SqlxSpan::query(DbSystem::Postgres, stmt);
//! let row = sqlx::query(stmt).bind(1).fetch_one(&pool).instrument(span).await?;
//! ```
//!
//! When the `otel` feature is also enabled the spans are picked up by the
//! tracing-opentelemetry layer installed by [`crate::otel_layer::init_with_otel`].

use tracing::{field::Empty, Span};

/// SQL dialect tag used as the `db.system` attribute on instrumented
/// query spans (matching the OpenTelemetry semantic conventions).
///
/// Marked `#[non_exhaustive]` so adding new dialects in a minor release
/// (e.g. `Mssql`) is non-breaking.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum DbSystem {
    /// PostgreSQL — emitted as `db.system = "postgresql"`.
    Postgres,
    /// MySQL / MariaDB — emitted as `db.system = "mysql"`.
    Mysql,
    /// SQLite — emitted as `db.system = "sqlite"`.
    Sqlite,
    /// Catch-all for any other SQL dialect — emitted as
    /// `db.system = "other_sql"`.
    Other,
}

impl DbSystem {
    /// Stable string representation used for the `db.system` span field.
    pub fn as_str(&self) -> &'static str {
        match self {
            DbSystem::Postgres => "postgresql",
            DbSystem::Mysql => "mysql",
            DbSystem::Sqlite => "sqlite",
            DbSystem::Other => "other_sql",
        }
    }
}

/// Span constructor for sqlx queries — see [`SqlxSpan::query`].
pub struct SqlxSpan;

impl SqlxSpan {
    /// Open a `tracing::Span` describing a sqlx query. Set
    /// `db.rows_affected` after execution via `Span::current().record(...)`.
    pub fn query(system: DbSystem, statement: &str) -> Span {
        let span = tracing::info_span!(
            "sqlx.query",
            otel.kind = "client",
            db.system = system.as_str(),
            db.statement = statement,
            db.rows_affected = Empty,
        );
        span
    }
}

/// Helper to record the `db.rows_affected` attribute on the current span
/// after a sqlx execute completes.
pub fn record_rows_affected(span: &Span, n: u64) {
    span.record("db.rows_affected", n);
}
