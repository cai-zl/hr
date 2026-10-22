//! hr boot 全家桶示例：Nacos 配置中心 + pg + redis + 服务注册 + 优雅停机。
//!
//! ```bash
//! export NACOS_SERVER_ADDR=127.0.0.1:8848
//! # 配置中心放 data_id=boot（group DEFAULT_GROUP）的 TOML：
//! #   port = 3000
//! #   database_url = "postgres://user:pass@127.0.0.1:5432/hr"
//! #   redis_url = "redis://127.0.0.1:6379"
//! cargo run --example boot
//! ```
//!
//! 业务模块的固定写法：零参函数返回 `Router<AppState>`；启动钩子是收 `AppState` 的
//! 普通异步函数，`serve(..).with_migrator(..).with_hook(..).await` 一行启动。
use axum::Router;
use axum::extract::{Path, State};
use axum::routing::get;
use hr::pg::sqlx;
use hr::{AppState, Hr, HrError, HrResult};
use serde_json::json;

fn user_routes() -> Router<AppState> {
    Router::new().route("/user/{id}", get(get_user))
}

async fn get_user(State(st): State<AppState>, Path(id): Path<i64>) -> HrResult<serde_json::Value> {
    let name: Option<(String,)> =
        sqlx::query_as::<_, (String,)>("select name from t_user where id = $1")
            .bind(id)
            .fetch_optional(&st.pg)
            .await
            .map_err(HrError::from)?;
    match name {
        Some((name,)) => Ok(Hr::ok(json!({ "id": id, "name": name }))),
        None => Err(HrError::new(1001, "user not found")),
    }
}

fn health_routes() -> Router<AppState> {
    Router::new().route("/health", get(health))
}

async fn health(State(st): State<AppState>) -> HrResult<serde_json::Value> {
    use hr::redis::deadpool_redis;
    let mut conn = st.redis.get().await.map_err(HrError::from)?;
    let pong: String = deadpool_redis::redis::cmd("PING")
        .query_async(&mut *conn)
        .await
        .map_err(HrError::from)?;
    Ok(Hr::ok(json!({ "app": st.config.app_name, "redis": pong })))
}

/// 启动钩子：普通异步函数，收 `AppState`，返回 Err 则不启动、不注册。
async fn on_start(st: AppState) -> Result<(), HrError> {
    tracing::info!(app = %st.config.app_name, "startup hook: seed / warm-up here");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), HrError> {
    tracing_subscriber::fmt::init();
    hr::boot::serve([user_routes(), health_routes()])
        .with_migrator(sqlx::migrate!("./migrations"))
        .with_hook(on_start)
        .await
}
