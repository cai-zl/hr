//! 0.2 模块测试：默认值、env 解析、配置解析、extra 段、错误转换。
use hr::boot::{self, Config};
use hr::nacos;
use hr::pg::sqlx;
use serde::Deserialize;

#[derive(Deserialize)]
struct Biz {
    timeout: u64,
}

#[test]
fn default_app_name_is_bare_name() {
    let name = boot::default_app_name();
    assert!(!name.is_empty());
    assert!(!name.contains('/'));
    assert!(!name.contains('\\'));
}

#[test]
fn from_env_requires_server_addr() {
    // edition 2024 起 env 变更标记为 unsafe；测试进程内执行到此，安全
    unsafe { std::env::remove_var("NACOS_SERVER_ADDR") };
    let err = nacos::from_env().unwrap_err();
    assert_eq!(err.code, 500);
    assert!(err.message.contains("NACOS_SERVER_ADDR"));
}

#[test]
fn config_defaults_and_extra() {
    let cfg: Config = nacos::parse_config(
        r#"
port = 8080
database_url = "postgres://u:p@127.0.0.1:5432/hr"
redis_url = "redis://127.0.0.1:6379"
timeout = 30
"#,
    )
    .unwrap();
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.host, "0.0.0.0");
    assert_eq!(cfg.redis.pool_size, 10);
    assert_eq!(cfg.pg.max_connections, 10);
    assert!(!cfg.app_name.is_empty());
    assert!(cfg.advertise_ip.is_none());
    let biz: Biz = cfg.extra().unwrap();
    assert_eq!(biz.timeout, 30);
}

#[test]
fn config_missing_required_field_fails() {
    let err = nacos::parse_config::<Config>("port = 8080").unwrap_err();
    assert_eq!(err.code, 500);
}

#[test]
fn parse_config_bad_toml_is_500() {
    let err = nacos::parse_config::<Config>("[[[not toml").unwrap_err();
    assert_eq!(err.code, 500);
}

#[test]
fn error_conversions_map_to_500() {
    let e: hr::HrError = sqlx::Error::RowNotFound.into();
    assert_eq!(e.code, 500);
    let e: hr::HrError = nacos_sdk::api::error::Error::NoAvailableServer.into();
    assert_eq!(e.code, 500);
    let io = std::io::Error::other("x");
    let e: hr::HrError = deadpool_redis::redis::RedisError::from(io).into();
    assert_eq!(e.code, 500);
    let io = std::io::Error::other("x");
    let e: hr::HrError =
        deadpool_redis::PoolError::Backend(deadpool_redis::redis::RedisError::from(io)).into();
    assert_eq!(e.code, 500);
}

#[test]
fn config_pg_redis_sections_default_and_override() {
    let c: Config = nacos::parse_config(
        "port = 8080\ndatabase_url = \"postgres://x\"\nredis_url = \"redis://x\"",
    )
    .unwrap();
    assert_eq!(c.pg.max_connections, 10);
    assert_eq!(c.pg.acquire_timeout_secs, 5);
    assert_eq!(c.redis.pool_size, 10);
    assert!(c.redis.wait_timeout_secs.is_none());

    let c: Config = nacos::parse_config(
        "port = 8080\ndatabase_url = \"p\"\nredis_url = \"r\"\n\n[pg]\nmax_connections = 20\nacquire_timeout_secs = 10\n\n[redis]\npool_size = 32\nwait_timeout_secs = 8",
    )
    .unwrap();
    assert_eq!(c.pg.max_connections, 20);
    assert_eq!(c.pg.acquire_timeout_secs, 10);
    assert_eq!(c.redis.pool_size, 32);
    assert_eq!(c.redis.wait_timeout_secs, Some(8));
}

#[test]
fn legacy_flat_redis_pool_size_falls_into_extra() {
    let c: Config = nacos::parse_config(
        "port = 8080\ndatabase_url = \"p\"\nredis_url = \"r\"\nredis_pool_size = 32",
    )
    .unwrap();
    assert!(c.extra.get("redis_pool_size").is_some());
}

#[test]
fn migrate_macro_embeds_files() {
    let m = sqlx::migrate!("./migrations");
    assert!(!m.migrations.is_empty());
}

#[test]
fn migrate_error_maps_to_500() {
    let e: hr::HrError =
        sqlx::migrate::MigrateError::Source(Box::new(sqlx::Error::RowNotFound)).into();
    assert_eq!(e.code, 500);
}
