//! PostgreSQL 连接池（feature = "pg"）。
//!
//! 配置归本模块所有：[`PgConfig`] 定义全部连接参数与默认值，
//! boot 等上层只负责把它从 TOML 传进来，不感知连接细节。

pub use sqlx;
pub use sqlx::migrate;

use serde::Deserialize;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use tracing::info;

/// pg 连接配置（TOML `[pg]` 段），字段全部有默认值，可整段省略。
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct PgConfig {
    /// 最大连接数，默认 10。
    pub max_connections: u32,
    /// 从池中获取连接的超时秒数，默认 5。
    pub acquire_timeout_secs: u64,
}

impl Default for PgConfig {
    fn default() -> Self {
        Self { max_connections: 10, acquire_timeout_secs: 5 }
    }
}

/// 连接 PostgreSQL，使用默认配置（max_connections = 10，acquire 超时 5s）。
pub async fn connect(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    connect_with(url, &PgConfig::default()).await
}

/// 连接 PostgreSQL，使用给定配置。
pub async fn connect_with(url: &str, cfg: &PgConfig) -> Result<sqlx::PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .acquire_timeout(Duration::from_secs(cfg.acquire_timeout_secs))
        .connect(url)
        .await;
    if pool.is_ok() {
        info!(max_connections = cfg.max_connections, "PostgreSQL pool connected");
    }
    pool
}
