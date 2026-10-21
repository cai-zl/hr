//! PostgreSQL 连接池（feature = "pg"）。

pub use sqlx;

use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

/// 连接 PostgreSQL，返回连接池（max_connections = 10，acquire 超时 5s）。
pub async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
}
