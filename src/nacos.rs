//! Nacos 客户端（feature = "nacos"）：配置中心 + 服务注册。
//!
//! 包装 `nacos-sdk` 0.3 的同步 API；`register`/`get_config` 只在启动、
//! 停机时各调用一次，同步开销可忽略。

pub use nacos_sdk;

use crate::HrError;
use nacos_sdk::api::config::{ConfigService, ConfigServiceBuilder};
use nacos_sdk::api::error::Error as NacosError;
use nacos_sdk::api::naming::{NamingService, NamingServiceBuilder, ServiceInstance};
use nacos_sdk::api::props::ClientProps;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;

/// 就绪的 Nacos 客户端（配置 + 注册），克隆只是引用计数。
#[derive(Clone)]
pub struct Nacos {
    config: Arc<dyn ConfigService + Send + Sync>,
    naming: Arc<dyn NamingService + Send + Sync>,
}

impl std::fmt::Debug for Nacos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nacos")
    }
}

/// 连接 Nacos（配置中心 + 命名服务一次建好）。
pub fn connect(server_addr: &str, namespace: &str) -> Result<Nacos, HrError> {
    connect_auth(server_addr, namespace, None, None)
}

fn connect_auth(
    server_addr: &str,
    namespace: &str,
    username: Option<String>,
    password: Option<String>,
) -> Result<Nacos, HrError> {
    let mut props = ClientProps::new()
        .server_addr(server_addr)
        .namespace(namespace);
    if let (Some(u), Some(p)) = (username, password) {
        props = props.auth_username(u).auth_password(p);
    }
    let config = ConfigServiceBuilder::new(props.clone()).build()?;
    let naming = NamingServiceBuilder::new(props).build()?;
    Ok(Nacos {
        config: Arc::new(config),
        naming: Arc::new(naming),
    })
}

/// 从环境变量连接：`NACOS_SERVER_ADDR`（必填）、`NACOS_NAMESPACE`（默认 public）、
/// `NACOS_USERNAME`/`NACOS_PASSWORD`（两者都设置才启用鉴权）。
pub fn from_env() -> Result<Nacos, HrError> {
    let server_addr = std::env::var("NACOS_SERVER_ADDR")
        .map_err(|_| HrError::msg("missing env: NACOS_SERVER_ADDR"))?;
    let namespace = std::env::var("NACOS_NAMESPACE").unwrap_or_else(|_| "public".into());
    let username = std::env::var("NACOS_USERNAME").ok();
    let password = std::env::var("NACOS_PASSWORD").ok();
    connect_auth(&server_addr, &namespace, username, password)
}

impl Nacos {
    /// 读取配置文本；配置不存在返回 `Ok(None)`，其余错误向上传播。
    pub fn get_config(&self, data_id: &str, group: &str) -> Result<Option<String>, HrError> {
        match self
            .config
            .get_config(data_id.to_string(), group.to_string())
        {
            Ok(resp) => Ok(Some(resp.content().clone())),
            Err(NacosError::ConfigNotFound(_)) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 读取并解析 TOML 配置；不存在返回 `Ok(None)`。
    pub fn load_config<T: DeserializeOwned>(
        &self,
        data_id: &str,
        group: &str,
    ) -> Result<Option<T>, HrError> {
        match self.get_config(data_id, group)? {
            Some(text) => Ok(Some(parse_config(&text)?)),
            None => Ok(None),
        }
    }

    /// 注册临时服务实例（进程退出或超时后 Nacos 自动摘除）。
    pub fn register(&self, service: &str, ip: &str, port: i32, group: &str) -> Result<(), HrError> {
        self.naming
            .register_instance(service.to_string(), Some(group.to_string()), instance(ip, port))?;
        Ok(())
    }

    /// 注销服务实例（boot 在停机时调用；临时实例即使漏调也会自动过期）。
    pub fn deregister(
        &self,
        service: &str,
        ip: &str,
        port: i32,
        group: &str,
    ) -> Result<(), HrError> {
        self.naming.deregister_instance(
            service.to_string(),
            Some(group.to_string()),
            instance(ip, port),
        )?;
        Ok(())
    }
}

/// 解析 TOML 配置文本。
pub fn parse_config<T: DeserializeOwned>(text: &str) -> Result<T, HrError> {
    toml::from_str(text).map_err(HrError::from)
}

fn instance(ip: &str, port: i32) -> ServiceInstance {
    ServiceInstance {
        instance_id: None,
        ip: ip.to_string(),
        port,
        weight: 1.0,
        healthy: true,
        enabled: true,
        ephemeral: true,
        cluster_name: None,
        service_name: None,
        metadata: HashMap::new(),
    }
}
