//! 一键启动（feature = "boot"）：配置 → 连接 → 注册 → 服务 → 停机反注册。

use crate::nacos::Nacos;
use crate::pg::sqlx;
use crate::redis::deadpool_redis;
use crate::{HrError, merge_all};
use axum::Router;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::net::IpAddr;
use std::path::Path;
use toml::Table;

/// 运行时二进制名（去路径），app_name 与 NACOS_DATA_ID 的缺省值。
pub fn default_app_name() -> String {
    std::env::args_os()
        .next()
        .as_deref()
        .map(Path::new)
        .and_then(Path::file_name)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hr-app".into())
}

fn default_host() -> String {
    "0.0.0.0".into()
}

fn default_pool_size() -> usize {
    10
}

/// 服务配置：来自 Nacos 配置中心（TOML），未声明的键全部落入 `extra`。
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_app_name")]
    pub app_name: String,
    #[serde(default = "default_host")]
    pub host: String,
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    #[serde(default = "default_pool_size")]
    pub redis_pool_size: usize,
    /// 对外注册 IP；缺省时自动探测，探测失败则启动报错。
    pub advertise_ip: Option<String>,
    /// 业务自定义配置段（未在上方声明的所有键）。
    #[serde(flatten)]
    pub extra: Table,
}

impl Config {
    /// 把 `extra` 段反序列化成业务自定义配置类型。
    pub fn extra<T: DeserializeOwned>(&self) -> Result<T, HrError> {
        T::deserialize(toml::Value::Table(self.extra.clone())).map_err(HrError::from)
    }
}

/// 全家桶服务状态：`run`/`run_env` 组装后交给各模块 handlers。
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pg: sqlx::PgPool,
    pub redis: deadpool_redis::Pool,
    pub nacos: Nacos,
}

/// 探测本机对外 IP（UDP connect 8.8.8.8:80，不实际发包）。
pub fn detect_ip() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    s.local_addr().ok().map(|a| a.ip())
}

/// 全环境启动：NACOS_* 环境变量 → Nacos 配置中心 → 连 pg/redis → 注册 → 服务 → 停机反注册。
pub async fn run_env(routes: impl IntoIterator<Item = Router<AppState>>) -> Result<(), HrError> {
    let nacos = crate::nacos::from_env()?;
    let data_id = std::env::var("NACOS_DATA_ID").unwrap_or_else(|_| default_app_name());
    let group = std::env::var("NACOS_GROUP").unwrap_or_else(|_| "DEFAULT_GROUP".into());
    let config: Config = nacos
        .load_config(&data_id, &group)?
        .ok_or_else(|| HrError::msg(format!("nacos config not found: {data_id}/{group}")))?;
    run(config, nacos, routes).await
}

/// 显式全参启动：调用方自带 config 与已连好的 nacos。
pub async fn run(
    config: Config,
    nacos: Nacos,
    routes: impl IntoIterator<Item = Router<AppState>>,
) -> Result<(), HrError> {
    let pg = crate::pg::connect(&config.database_url).await?;
    let redis = crate::redis::connect(&config.redis_url, config.redis_pool_size)?;
    start(config, pg, redis, nacos, routes).await
}

async fn start(
    config: Config,
    pg: sqlx::PgPool,
    redis: deadpool_redis::Pool,
    nacos: Nacos,
    routes: impl IntoIterator<Item = Router<AppState>>,
) -> Result<(), HrError> {
    let advertise = match config.advertise_ip.clone() {
        Some(ip) => ip,
        None => detect_ip()
            .map(|a| a.to_string())
            .ok_or_else(|| HrError::msg("无法探测本机 IP，请在配置中显式设置 advertise_ip"))?,
    };
    let service = config.app_name.clone();
    let port = config.port as i32;
    let host = config.host.clone();
    let bind_port = config.port;
    let group = std::env::var("NACOS_GROUP").unwrap_or_else(|_| "DEFAULT_GROUP".into());

    let app = merge_all(routes).with_state(AppState {
        config,
        pg,
        redis,
        nacos: nacos.clone(),
    });
    nacos.register(&service, &advertise, port, &group)?;

    let listener = tokio::net::TcpListener::bind((host.as_str(), bind_port))
        .await
        .map_err(|e| HrError::msg(format!("bind {host}:{bind_port} failed: {e}")))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| HrError::msg(e.to_string()))?;

    // 临时实例，注销失败也会被 Nacos 自动摘除
    let _ = nacos.deregister(&service, &advertise, port, &group);
    Ok(())
}

/// ctrl_c / SIGTERM 优雅停机。
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl_c handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
