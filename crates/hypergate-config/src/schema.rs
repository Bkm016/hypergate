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
    /// 同时处理入站 TCP 连接的最大数量。超出上限的新连接会被立即关闭,
    /// 避免慢速/空闲连接耗尽 task 和 socket 资源。`0` 表示显式禁用上限
    /// (仅当部署已在外层施加连接限制时才建议使用)。
    pub max_connections: usize,
    /// HTTP/1 单连接等待请求行+请求头整体读取的时间上限。
    /// 慢首部或空闲连接超过该时间会被 hyper 主动关闭,不能无限占用 task。
    /// `0` 表示禁用(仅当外部显式配置时才建议禁用)。
    pub header_read_timeout: Duration,
    /// 与 version app 建立 TCP/TLS 连接的超时。超时则快速失败返回 502,
    /// 避免挂起的 version app 拖垮 gateway。
    pub version_connect_timeout: Duration,
    /// 等待 version app 响应头到达的截止时间。覆盖从发起 `Client::request`
    /// (含连接建立与请求发送)到响应头返回的整个阶段;响应 body / SSE 流
    /// 不受此限制,避免切断长响应。
    pub version_response_header_timeout: Duration,
}

impl Default for ServerConfig {
    /// 默认监听本机 8080,并对慢首部/超额连接启用保护性超时。
    fn default() -> Self {
        Self {
            listen: SocketAddr::from(([127, 0, 0, 1], 8080)),
            // 4096 给普通流量留足余量,同时挡住连接洪峰;`0` 保留为显式禁用语义。
            max_connections: 4096,
            // 30 秒足够覆盖正常客户端发送请求头,同时挡住慢首部攻击。
            header_read_timeout: Duration::from_secs(30),
            // 5 秒连接超时足以发现不可达 version app,避免长时间挂起。
            version_connect_timeout: Duration::from_secs(5),
            // 30 秒响应头等待覆盖绝大多数普通请求,SSE 的首字节通常也在此之内。
            version_response_header_timeout: Duration::from_secs(30),
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
