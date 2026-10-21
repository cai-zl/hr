//! hr — unified HTTP response envelope for axum.
//!
//! Every handler returns the same shape, so AI-written services stay consistent:
//!
//! ```json
//! {"code":0,"message":"ok","data":{"id":1}}
//! {"code":404,"message":"Not Found"}
//! ```
//!
//! HTTP status is always 200; success/failure is read from `code` (0 = ok).
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

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;


#[cfg(feature = "pg")]
pub mod pg;
#[cfg(feature = "redis")]
pub mod redis;
#[cfg(feature = "nacos")]
pub mod nacos;
#[cfg(feature = "boot")]
pub mod boot;

#[cfg(feature = "boot")]
pub use boot::AppState;

/// 合并各业务模块的路由：`merge_all([user::routes(), order::routes()])`。
pub fn merge_all<S>(routers: impl IntoIterator<Item = axum::Router<S>>) -> axum::Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    routers
        .into_iter()
        .fold(axum::Router::new(), |acc, router| acc.merge(router))
}
/// Success envelope: `{"code":0,"message":"ok","data":...}`.
#[derive(Serialize)]
pub struct Hr<T> {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
}

impl<T: Serialize> Hr<T> {
    pub fn ok(data: T) -> Self {
        Self { code: 0, message: "ok".into(), data: Some(data) }
    }

    /// Success with a custom message, e.g. "created".
    pub fn ok_with(data: T, message: impl Into<String>) -> Self {
        Self { code: 0, message: message.into(), data: Some(data) }
    }

    /// Business failure; `data` is omitted from the body.
    pub fn err(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
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
/// Error type for `?` in handlers; converts into an error envelope.
#[derive(Debug)]
pub struct HrError {
    pub code: i32,
    pub message: String,
}

impl HrError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into() }
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

/// Paginated payload: `{"list":[...],"total":n,"page":n,"page_size":n}`.
#[derive(Serialize)]
pub struct HrPage<T> {
    list: Vec<T>,
    total: u64,
    page: u64,
    page_size: u64,
}

impl<T> HrPage<T> {
    pub fn new(list: Vec<T>, total: u64, page: u64, page_size: u64) -> Self {
        Self { list, total, page, page_size }
    }
}

impl From<StatusCode> for HrError {
    fn from(status: StatusCode) -> Self {
        Self::new(status.as_u16() as i32, status.canonical_reason().unwrap_or("error"))
    }
}

impl From<anyhow::Error> for HrError {
    fn from(e: anyhow::Error) -> Self {
        Self::msg(e.to_string())
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

#[cfg(feature = "pg")]
impl From<sqlx::Error> for HrError {
    fn from(e: sqlx::Error) -> Self {
        Self::msg(e.to_string())
    }
}

#[cfg(feature = "redis")]
impl From<deadpool_redis::redis::RedisError> for HrError {
    fn from(e: deadpool_redis::redis::RedisError) -> Self {
        Self::msg(e.to_string())
    }
}

#[cfg(feature = "redis")]
impl From<deadpool_redis::PoolError> for HrError {
    fn from(e: deadpool_redis::PoolError) -> Self {
        Self::msg(e.to_string())
    }
}

#[cfg(feature = "redis")]
impl From<deadpool_redis::CreatePoolError> for HrError {
    fn from(e: deadpool_redis::CreatePoolError) -> Self {
        Self::msg(e.to_string())
    }
}

#[cfg(feature = "nacos")]
impl From<nacos_sdk::api::error::Error> for HrError {
    fn from(e: nacos_sdk::api::error::Error) -> Self {
        Self::msg(e.to_string())
    }
}

#[cfg(feature = "boot")]
impl From<toml::de::Error> for HrError {
    fn from(e: toml::de::Error) -> Self {
        Self::msg(e.to_string())
    }
}
