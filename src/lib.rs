//! hr — unified HTTP response envelope for axum.
//!
//! Every handler returns the same shape, so AI-written services stay consistent:
//!
//! ```json
//! {"code":0,"message":"ok","data":{"id":1}}
//! {"code":404,"message":"Not Found"}
//! ```
//!
//! HTTP status is always 200 for business responses; success/failure is read from `code` (0 = ok).
//! Framework errors (404/405/…) keep their real status code; empty bodies are
//! wrapped in an envelope, non-empty bodies pass through untouched.
//!
//! ```
//! use axum::{Router, routing::get};
//! use hr::{Hr, HrResult};
//!
//! #[derive(serde::Serialize)]
//! struct User { id: u32, name: String }
//!
//! async fn get_user() -> HrResult<User> {
//!     let user = User { id: 1, name: "alice".into() };
//!     Ok(Hr::ok(user))
//! }
//!
//! let app: Router = Router::new().route("/user", get(get_user));
//! ```

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[cfg(feature = "boot")]
pub mod boot;
#[cfg(feature = "boot")]
pub mod nacos;
#[cfg(feature = "boot")]
pub mod pg;
#[cfg(feature = "boot")]
pub mod redis;

#[cfg(feature = "boot")]
pub use boot::AppState;

/// 合并各业务模块的路由：`merge_all([user::routes(), order::routes()])`。
/// 框架错误（≥400，如 404/405/400）保留真实状态码，body 统一包装成信封。
pub fn merge_all<S>(routers: impl IntoIterator<Item = axum::Router<S>>) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    routers
        .into_iter()
        .fold(axum::Router::new(), |acc, router| acc.merge(router))
        .layer(axum::middleware::from_fn(framework_error_envelope))
}

/// axum 框架错误包装：空 body 的 ≥400 响应保留真实状态码并包装成信封；
/// body 非空的响应（业务自写错误页）连同原状态码、原 content-type 原样透传。
async fn framework_error_envelope(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let res = next.run(req).await;
    let status = res.status();
    if !status.is_client_error() && !status.is_server_error() {
        return res;
    }
    let (parts, body) = res.into_parts();
    let bytes = axum::body::to_bytes(body, 64 * 1024).await;
    match bytes {
        // 空 body = axum 框架错误（404/405 等）：真状态码 + 信封
        Ok(b) if b.is_empty() => {
            let message = parts
                .status
                .canonical_reason()
                .unwrap_or("error")
                .to_owned();
            (
                parts.status,
                Hr::<String>::err(parts.status.as_u16() as i32, message),
            )
                .into_response()
        }
        // 读不出内容：原样透传（头/状态码保留），不伪造信封
        Err(_) => Response::from_parts(parts, axum::body::Body::empty()),
        // body 非空 = 业务自写的错误页：原状态码 + 原头（含 content-type）原样透传
        Ok(b) => Response::from_parts(parts, axum::body::Body::from(b)),
    }
}
/// Success envelope: `{"code":0,"message":"ok","data":...}`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hr<T> {
    code: i32,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

impl<T: Serialize> Hr<T> {
    pub fn ok(data: T) -> Self {
        Self {
            code: 0,
            message: "ok".into(),
            data: Some(data),
        }
    }

    /// Success with a custom message, e.g. "created".
    pub fn ok_with(data: T, message: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: message.into(),
            data: Some(data),
        }
    }

    /// Business failure; `data` is omitted from the body.
    ///
    /// `code = 0` means success in this protocol; `err(0, ..)` is treated as 500.
    pub fn err(code: i32, message: impl Into<String>) -> Self {
        let code = if code == 0 { 500 } else { code };
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Paginated success: `data = {"list":[...],"total":n,"page":n,"page_size":n}`.
    pub fn page(list: Vec<T>, total: u64, page: u64, page_size: u64) -> Hr<HrPage<T>> {
        Hr::ok(HrPage::new(list, total, page, page_size))
    }
}

impl<T: Serialize> IntoResponse for Hr<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

/// Error type for `?` in handlers; converts into an error envelope.
#[derive(Debug)]
pub struct HrError {
    pub code: i32,
    pub message: String,
}

impl HrError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// `code = 500`.
    pub fn msg(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }
}

impl IntoResponse for HrError {
    fn into_response(self) -> Response {
        Hr::<serde_json::Value>::err(self.code, self.message).into_response()
    }
}

/// Handler return type: `Ok(Hr::ok(data))` wraps the payload, `Err(...)`/`?`
/// produces an error envelope. Covered by axum's blanket impl for `Result`.
///
/// Accepted `Err` sources via `From`: `HrError`, `StatusCode`, `anyhow::Error`,
/// `String`, `&str` — so `let u = repo.find(id)?;` just works.
pub type HrResult<T> = Result<Hr<T>, HrError>;

/// Paginated payload: `{"list":[...],"total":n,"page":n,"pageSize":n}`.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HrPage<T> {
    list: Vec<T>,
    total: u64,
    page: u64,
    page_size: u64,
}

impl<T> HrPage<T> {
    pub fn new(list: Vec<T>, total: u64, page: u64, page_size: u64) -> Self {
        Self {
            list,
            total,
            page,
            page_size,
        }
    }
}

impl From<StatusCode> for HrError {
    fn from(status: StatusCode) -> Self {
        Self::new(
            status.as_u16() as i32,
            status.canonical_reason().unwrap_or("error"),
        )
    }
}

impl From<anyhow::Error> for HrError {
    fn from(e: anyhow::Error) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}

impl From<String> for HrError {
    fn from(message: String) -> Self {
        Self::msg(message)
    }
}

impl From<&str> for HrError {
    fn from(message: &str) -> Self {
        Self::msg(message)
    }
}

#[cfg(feature = "boot")]
impl From<sqlx::Error> for HrError {
    fn from(e: sqlx::Error) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}
#[cfg(feature = "boot")]
impl From<sqlx::migrate::MigrateError> for HrError {
    fn from(e: sqlx::migrate::MigrateError) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}

#[cfg(feature = "boot")]
impl From<deadpool_redis::redis::RedisError> for HrError {
    fn from(e: deadpool_redis::redis::RedisError) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}

#[cfg(feature = "boot")]
impl From<deadpool_redis::PoolError> for HrError {
    fn from(e: deadpool_redis::PoolError) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}

#[cfg(feature = "boot")]
impl From<deadpool_redis::CreatePoolError> for HrError {
    fn from(e: deadpool_redis::CreatePoolError) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}

#[cfg(feature = "boot")]
impl From<nacos_sdk::api::error::Error> for HrError {
    fn from(e: nacos_sdk::api::error::Error) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}

#[cfg(feature = "boot")]
impl From<toml::de::Error> for HrError {
    fn from(e: toml::de::Error) -> Self {
        warn!(error = ?e, "infrastructure error mapped to 500");
        Self::msg("internal error")
    }
}
