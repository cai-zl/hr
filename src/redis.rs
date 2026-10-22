//! Redis 连接池（属于 boot 全家桶），基于 deadpool-redis。
//!
//! [`RedisConfig`] 定义 `[redis]` 配置段的全部键与默认值；连接动作由
//! `hr::boot::serve` 在启动时执行，业务代码不直接调用（函数为 crate 内部）。

pub use deadpool_redis;

use serde::Deserialize;
use std::time::Duration;
use tracing::info;

/// redis 连接配置（TOML `[redis]` 段），字段全部有默认值，可整段省略。
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RedisConfig {
    /// 池大小，默认 10。
    pub pool_size: usize,
    /// 等待空闲连接的超时秒数，默认 None（deadpool 库默认，不设超时）。
    pub wait_timeout_secs: Option<u64>,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            pool_size: 10,
            wait_timeout_secs: None,
        }
    }
}

/// 连接 Redis，使用给定配置（crate 内部，由 boot 调用）。
pub(crate) fn connect_with(
    url: &str,
    cfg: &RedisConfig,
) -> Result<deadpool_redis::Pool, deadpool_redis::CreatePoolError> {
    let mut pool_cfg = deadpool_redis::PoolConfig::new(cfg.pool_size);
    if let Some(secs) = cfg.wait_timeout_secs {
        pool_cfg.timeouts.wait = Some(Duration::from_secs(secs));
    }
    let mut dcfg = deadpool_redis::Config::from_url(url);
    dcfg.pool = Some(pool_cfg);
    let pool = dcfg.create_pool(Some(deadpool_redis::Runtime::Tokio1));
    if pool.is_ok() {
        info!(pool_size = cfg.pool_size, "Redis pool connected");
    }
    pool
}
