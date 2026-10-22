//! Redis 连接池（feature = "redis"），基于 deadpool-redis。
//!
//! 配置归本模块所有：[`RedisConfig`] 定义全部池参数与默认值，
//! boot 等上层只负责把它从 TOML 传进来，不感知连接细节。

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

/// 连接 Redis，只指定池大小（等待超时用库默认）。
pub fn connect(
    url: &str,
    max_size: usize,
) -> Result<deadpool_redis::Pool, deadpool_redis::CreatePoolError> {
    connect_with(
        url,
        &RedisConfig {
            pool_size: max_size,
            ..Default::default()
        },
    )
}

/// 连接 Redis，使用给定配置。
pub fn connect_with(
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
