//! 一键启动（feature = "boot"）：配置 → 连接 → 注册 → 服务 → 停机反注册。

use crate::nacos::Nacos;
use crate::pg::sqlx;
use crate::redis::deadpool_redis;
use crate::{HrError, merge_all};
use axum::Router;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::net::IpAddr;
use std::path::Path;
use std::pin::Pin;
use toml::Table;

use tracing::{debug, error, info};
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

fn default_group() -> String {
    "DEFAULT_GROUP".into()
}

/// 服务配置：来自 Nacos 配置中心（TOML），未声明的键全部落入 `extra`。
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_app_name")]
    pub app_name: String,
    #[serde(default = "default_host")]
    pub host: String,
    /// Nacos 服务注册分组，默认 DEFAULT_GROUP。
    #[serde(default = "default_group")]
    pub group: String,
    pub port: u16,
    pub database_url: String,
    pub redis_url: String,
    /// pg 连接配置（`[pg]` 段，全默认可省略）。
    #[serde(default)]
    pub pg: crate::pg::PgConfig,
    /// redis 连接配置（`[redis]` 段，全默认可省略）。
    #[serde(default)]
    pub redis: crate::redis::RedisConfig,
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

/// 全家桶服务状态：`boot::serve` 组装后交给各模块 handlers；启动钩子按值收到它的 Clone。
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

// 装箱形态仅存于库内：钩子逐个经 with_hook 擦除为同一类型（孤儿规则不允许一揽子
// From<F>，且数组字面量要求元素同类型——不同 fn item 类型各异，无法装进同一个数组）。
type StartupHook =
    Box<dyn FnOnce(AppState) -> Pin<Box<dyn Future<Output = Result<(), HrError>> + Send>> + Send>;

/// 启动来源：全环境变量读取，或显式传入 config 与 nacos。
enum BootSource {
    Env,
    /// Box 避免 enum 尺寸被 (Config, Nacos) 撑大（clippy large_enum_variant）。
    Explicit(Box<(Config, Nacos)>),
}

/// 一键启动的构建器：`serve(..).with_migrator(..).with_hook(..).await`。
pub struct Boot {
    source: BootSource,
    routes: Vec<Router<AppState>>,
    migrator: Option<sqlx::migrate::Migrator>,
    hooks: Vec<StartupHook>,
}

/// 全环境启动：NACOS_* 环境变量 → Nacos 配置中心 → 迁移 → 连 pg/redis → 钩子 → 注册 → 服务 → 停机反注册。
pub fn serve(routes: impl IntoIterator<Item = Router<AppState>>) -> Boot {
    Boot {
        source: BootSource::Env,
        routes: routes.into_iter().collect(),
        migrator: None,
        hooks: Vec::new(),
    }
}

/// 显式启动：调用方自带 config 与已连好的 nacos。
pub fn serve_with(
    config: Config,
    nacos: Nacos,
    routes: impl IntoIterator<Item = Router<AppState>>,
) -> Boot {
    Boot {
        source: BootSource::Explicit(Box::new((config, nacos))),
        routes: routes.into_iter().collect(),
        migrator: None,
        hooks: Vec::new(),
    }
}

impl Boot {
    /// 挂接编译期内嵌的迁移（`sqlx::migrate!("./migrations")`），不调用则不跑迁移。
    pub fn with_migrator(mut self, migrator: sqlx::migrate::Migrator) -> Self {
        self.migrator = Some(migrator);
        self
    }

    /// 追加一个启动钩子，可链式多次调用，按调用顺序执行。
    ///
    /// 传普通异步函数即可（收 `AppState`，返回 `HrResult<()>`）；常驻任务在钩子里
    /// `tokio::spawn`。不同函数需逐个调用（数组无法容纳类型各异的 fn item）。
    pub fn with_hook<F, Fut>(mut self, hook: F) -> Self
    where
        F: FnOnce(AppState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), HrError>> + Send + 'static,
    {
        self.hooks.push(Box::new(move |st| Box::pin(hook(st))));
        self
    }

    async fn start(self) -> Result<(), HrError> {
        let Boot {
            source,
            routes,
            migrator,
            hooks,
        } = self;
        let (config, nacos) = match source {
            BootSource::Env => {
                let nacos = crate::nacos::from_env()?;
                let data_id = std::env::var("NACOS_DATA_ID").unwrap_or_else(|_| default_app_name());
                let group = std::env::var("NACOS_GROUP").unwrap_or_else(|_| "DEFAULT_GROUP".into());
                debug!(%data_id, %group, "loading config from nacos");
                let config: Config = nacos.load_config(&data_id, &group)?.ok_or_else(|| {
                    HrError::msg(format!("nacos config not found: {data_id}/{group}"))
                })?;
                (config, nacos)
            }
            BootSource::Explicit(boxed) => *boxed,
        };
        info!(app = %config.app_name, "starting");
        let result = async {
            let pg = crate::pg::connect_with(&config.database_url, &config.pg).await?;
            if let Some(migrator) = migrator {
                migrator.run(&pg).await?;
                info!("migrations applied");
            }
            let redis = crate::redis::connect_with(&config.redis_url, &config.redis)?;
            run_app(config, pg, redis, nacos, routes, hooks).await
        }
        .await;
        if let Err(e) = &result {
            error!(?e, "startup failed");
        }
        result
    }
}

impl IntoFuture for Boot {
    type Output = Result<(), HrError>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<(), HrError>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.start())
    }
}

async fn run_app(
    config: Config,
    pg: sqlx::PgPool,
    redis: deadpool_redis::Pool,
    nacos: Nacos,
    routes: Vec<Router<AppState>>,
    hooks: Vec<StartupHook>,
) -> Result<(), HrError> {
    let advertise = match config.advertise_ip.clone() {
        Some(ip) => ip,
        None => detect_ip()
            .map(|a| a.to_string())
            .ok_or_else(|| HrError::msg("cannot detect local ip; set advertise_ip in config"))?,
    };
    let service = config.app_name.clone();
    let port = config.port as i32;
    let host = config.host.clone();
    let bind_port = config.port;
    let group = config.group.clone();
    let state = AppState {
        config,
        pg,
        redis,
        nacos: nacos.clone(),
    };
    for (i, hook) in hooks.into_iter().enumerate() {
        debug!(hook = i, "running startup hook");
        hook(state.clone()).await?;
    }

    let app = merge_all(routes).with_state(state);
    let listener = tokio::net::TcpListener::bind((host.as_str(), bind_port))
        .await
        .map_err(|e| HrError::msg(format!("bind {host}:{bind_port} failed: {e}")))?;
    info!(host, port = bind_port, "listening");
    nacos.register(&service, &advertise, port, &group)?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| HrError::msg(e.to_string()))?;

    // 临时实例，注销失败也会被 Nacos 自动摘除
    if let Err(e) = nacos.deregister(&service, &advertise, port, &group) {
        error!(?e, "deregister failed (ephemeral instance will expire)");
    }
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
    info!("shutdown signal received");
}
#[cfg(test)]
mod tests {
    use super::*;

    async fn hook_a(_st: AppState) -> Result<(), HrError> {
        Ok(())
    }

    async fn hook_b(_st: AppState) -> Result<(), HrError> {
        Ok(())
    }

    #[test]
    fn hooks_append_in_order_and_async_fns_coerce() {
        let boot =
            serve([])
                .with_hook(hook_a)
                .with_hook(hook_b)
                .with_hook(|st: AppState| async move {
                    let _ = st.config.app_name;
                    Ok(())
                });
        assert_eq!(boot.hooks.len(), 3);
    }
}
