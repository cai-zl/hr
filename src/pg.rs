//! PostgreSQL 连接池（属于 boot 全家桶）。
//!
//! [`PgConfig`] 定义 `[pg]` 配置段的全部键与默认值；连接动作由
//! `hr::boot::serve` 在启动时执行，业务代码不直接调用（函数为 crate 内部）。
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
        Self {
            max_connections: 10,
            acquire_timeout_secs: 5,
        }
    }
}

/// 连接 PostgreSQL，使用给定配置（crate 内部，由 boot 调用）。
pub(crate) async fn connect_with(url: &str, cfg: &PgConfig) -> Result<sqlx::PgPool, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .acquire_timeout(Duration::from_secs(cfg.acquire_timeout_secs))
        .connect(url)
        .await;
    if pool.is_ok() {
        info!(
            max_connections = cfg.max_connections,
            "PostgreSQL pool connected"
        );
    }
    pool
}
