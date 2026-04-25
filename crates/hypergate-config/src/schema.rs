//! 运行时配置 schema。这里只描述底座需要理解的通用字段。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use hypergate_core::{ConfigRevision, HyperError, HyperResult, VersionId};

/// 对外 HTTP 服务配置。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// HyperGate 对外监听地址。
    pub listen: SocketAddr,
}

impl Default for ServerConfig {
    /// 默认监听本机 8080。
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
        }
    }
}

/// 文件监听配置。
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// 是否启用配置文件监听。
    pub enabled: bool,
    /// 文件变更合并延迟。
    pub debounce: Duration,
}

impl Default for WatchConfig {
    /// 默认启用文件监听并合并短时间内的重复变更。
    fn default() -> Self {
        Self {
            enabled: true,
            debounce: Duration::from_millis(500),
        }
    }
}

/// 长连接排水配置。
#[derive(Debug, Clone)]
pub struct DrainConfig {
    /// draining 版本最多等待已有连接的时间。
    pub timeout: Duration,
    /// 超时后是否强制关闭仍在运行的流式请求。
    pub force_close_streams: bool,
}

impl Default for DrainConfig {
    /// 默认给长连接三十分钟排水时间且不强制关闭。
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30 * 60),
            force_close_streams: false,
        }
    }
}

/// 单个运行版本配置。
#[derive(Debug, Clone)]
pub struct VersionConfig {
    /// Version app HTTP 入口地址。
    pub endpoint: String,
    /// 健康检查地址。运行时可以据此扩展探活策略。
    pub health: Option<String>,
}

/// HyperGate 运行时配置。
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// 当前配置修订号。
    pub revision: ConfigRevision,
    /// 对外 HTTP 服务配置。
    pub server: ServerConfig,
    /// 默认接收新请求的版本。
    pub active_version: VersionId,
    /// 多版本 endpoint 配置。
    pub versions: HashMap<VersionId, VersionConfig>,
    /// 文件监听配置。
    pub watch: WatchConfig,
    /// 长连接排水配置。
    pub drain: DrainConfig,
}

impl RuntimeConfig {
    /// 创建一个最小可运行配置。
    pub fn minimal() -> Self {
        Self {
            revision: ConfigRevision::INITIAL,
            server: ServerConfig::default(),
            active_version: VersionId::new("v1"),
            versions: HashMap::new(),
            watch: WatchConfig::default(),
            drain: DrainConfig::default(),
        }
    }

    /// 返回当前 active 版本配置。
    pub fn active_version_config(&self) -> HyperResult<&VersionConfig> {
        self.versions
            .get(&self.active_version)
            .ok_or_else(|| HyperError::new("active version config not found"))
    }
}

/// 配置重载触发来源。
#[derive(Debug, Clone)]
pub enum ReloadTrigger {
    /// 文件监听触发。
    FileWatch {
        /// 发生变更的文件路径。
        path: PathBuf,
    },
    /// 管理指令触发。
    Command {
        /// 指令来源描述。
        source: String,
    },
}
