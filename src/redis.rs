//! Redis 连接池（feature = "redis"），基于 deadpool-redis。

pub use deadpool_redis;

/// 连接 Redis，返回指定大小的连接池。
pub fn connect(
    url: &str,
    max_size: usize,
) -> Result<deadpool_redis::Pool, deadpool_redis::CreatePoolError> {
    let mut cfg = deadpool_redis::Config::from_url(url);
    cfg.pool = Some(deadpool_redis::PoolConfig::new(max_size));
    cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
}
