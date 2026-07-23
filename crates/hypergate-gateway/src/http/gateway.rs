//! HTTP 网关核心。这里维护 active 版本快照并绑定版本租约。

use std::sync::Arc;

use crate::runtime::{VersionLease, VersionRegistry, VersionRuntime};
use arc_swap::ArcSwap;
use http::uri::{Authority, Scheme};
use hypergate_config::{RuntimeConfig, ServerConfig};
use hypergate_core::{HyperError, HyperResult, RequestKind};

use super::{ProxyBodyPolicy, RequestKindClassifier, VersionClients};

/// 已完成版本绑定的反代请求。
pub(crate) struct PreparedProxyRequest {
    /// 版本租约。
    pub(crate) lease: VersionLease,
    /// Version app 入口地址。
    pub(crate) endpoint: PreparedEndpoint,
}

/// 已预解析的 version endpoint。
#[derive(Clone)]
pub(crate) struct PreparedEndpoint {
    /// URI scheme。
    pub(crate) scheme: Scheme,
    /// URI authority。
    pub(crate) authority: Authority,
    /// endpoint 自带 path 前缀。
    pub(crate) base_path: Arc<str>,
}

/// 当前 active version 的热路径快照。
pub(crate) struct ActiveVersionTarget {
    /// Version app 入口地址。
    pub(crate) endpoint: PreparedEndpoint,
    /// 版本运行态。
    pub(crate) runtime: Arc<VersionRuntime>,
}

/// HyperGate HTTP 网关核心。
pub(crate) struct Gateway {
    /// 版本注册表。
    pub(crate) versions: Arc<VersionRegistry>,
    /// 当前 active version 热路径快照。
    pub(crate) active: ArcSwap<ActiveVersionTarget>,
}

impl Gateway {
    /// 创建网关核心。
    pub(crate) fn new(config: &RuntimeConfig, versions: Arc<VersionRegistry>) -> HyperResult<Self> {
        let active = Self::build_active_target(config, &versions)?;
        Ok(Self {
            versions,
            active: ArcSwap::from_pointee(active),
        })
    }

    /// 预构建 active version 热路径快照,由控制层一次性刷新热路径。
    /// 构建过程不立即影响请求路由。
    pub(crate) fn prepare_active(
        &self,
        config: &RuntimeConfig,
    ) -> HyperResult<Arc<ActiveVersionTarget>> {
        Ok(Arc::new(Self::build_active_target(config, &self.versions)?))
    }

    /// 原子替换 active version 热路径快照并返回旧快照。
    pub(crate) fn swap_active(&self, active: Arc<ActiveVersionTarget>) -> Arc<ActiveVersionTarget> {
        self.active.swap(active)
    }

    /// 准备处理普通反代请求。切换后新请求读取新的 active 版本,旧请求继续持有旧版本租约。
    pub(crate) fn prepare_proxy(&self, kind: RequestKind) -> HyperResult<PreparedProxyRequest> {
        let active = self.active.load_full();
        match active.runtime.lease(kind) {
            Ok(lease) => Ok(PreparedProxyRequest {
                lease,
                endpoint: active.endpoint.clone(),
            }),
            Err(error) => {
                let current = self.active.load_full();
                if Arc::ptr_eq(&active, &current) {
                    return Err(error);
                }
                // 请求已读取旧 target 且旧版本随后进入 draining 时,仅对当前
                // target 重试一次,避免正常切换制造瞬时 502 或无限重试。
                let lease = current.runtime.lease(kind)?;
                Ok(PreparedProxyRequest {
                    lease,
                    endpoint: current.endpoint.clone(),
                })
            }
        }
    }

    /// 根据配置构建可被热路径原子替换的 active version 快照。
    fn build_active_target(
        config: &RuntimeConfig,
        versions: &Arc<VersionRegistry>,
    ) -> HyperResult<ActiveVersionTarget> {
        let endpoint = prepare_endpoint(&config.active_version_config()?.endpoint)?;
        let runtime = versions.ensure(config.active_version.clone())?;
        Ok(ActiveVersionTarget { endpoint, runtime })
    }
}

/// 预解析 version app 入口地址,避免每个请求重复拆 URI。
fn prepare_endpoint(endpoint: &str) -> HyperResult<PreparedEndpoint> {
    let uri = endpoint
        .parse::<http::Uri>()
        .map_err(|e| HyperError::new(format!("invalid version endpoint: {e}")))?;
    let scheme = uri
        .scheme()
        .cloned()
        .ok_or_else(|| HyperError::new("version endpoint missing scheme"))?;
    let authority = uri
        .authority()
        .cloned()
        .ok_or_else(|| HyperError::new("version endpoint missing authority"))?;
    let base_path = match uri.path() {
        "" | "/" => Arc::from(""),
        path => Arc::from(path.trim_end_matches('/')),
    };
    Ok(PreparedEndpoint {
        scheme,
        authority,
        base_path,
    })
}

/// HTTP 服务运行状态。
#[derive(Clone)]
pub(crate) struct HttpState {
    /// 网关核心。
    pub(crate) gateway: Arc<Gateway>,
    /// Version app HTTP 客户端池。
    pub(crate) clients: VersionClients,
    /// 请求连接类型识别器。
    pub(crate) classifier: Arc<dyn RequestKindClassifier>,
    /// 反代请求体策略。
    pub(crate) body_policy: ProxyBodyPolicy,
    /// 对外 HTTP 服务配置。直接持有配置快照,避免重复字段或无职责 wrapper;
    /// `proxy::serve` 据此施加连接上限与请求头超时,`forward_request` 据此
    /// 施加 version app 响应头等待。
    pub(crate) server: ServerConfig,
}
