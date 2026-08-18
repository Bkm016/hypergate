//! Version app HTTP 客户端池。

use std::time::Duration;

use axum::body::Body;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use hypergate_core::RequestKind;
use hypergate_core::{HyperError, HyperResult};

/// Version app HTTP 客户端。
pub(crate) type VersionClient = Client<HttpConnector, Body>;

/// 分离普通请求和流式请求的 version app 客户端池。
#[derive(Clone)]
pub(crate) struct VersionClients {
    /// 普通短请求客户端。允许保留空闲连接以提高吞吐。
    pub(crate) unary: VersionClient,
    /// 流式请求客户端。与短请求连接池隔离,避免长连接占用短请求池。
    pub(crate) streaming: VersionClient,
    /// WebSocket 客户端。Upgrade 语义只在 HTTP/1.1 成立,与数据面池隔离。
    pub(crate) websocket: VersionClient,
}

impl VersionClients {
    /// 构建 version app 客户端池。`connect_timeout` 仅约束与 version app 的
    /// 连接建立阶段;响应头等待由调用侧按请求施加,避免影响 body 流。
    pub(crate) fn new(connect_timeout: Duration) -> Self {
        // 转发热路径使用 hyper client,避免 reqwest 的通用客户端抽象额外开销。
        let unary = build_version_client(connect_timeout);
        let streaming = build_version_client(connect_timeout);
        let websocket = build_version_client(connect_timeout);
        Self {
            unary,
            streaming,
            websocket,
        }
    }

    /// 根据请求类型选择 version app 客户端。
    pub(crate) fn select(&self, kind: RequestKind) -> &VersionClient {
        match kind {
            RequestKind::Unary => &self.unary,
            RequestKind::Stream => &self.streaming,
        }
    }
}

/// version app 连接池每 host 保留的最大空闲连接数。
/// HTTP/1.1 下每请求独立连接，pool 容量直接决定并发上限；拉到 65535，
/// 慢流式请求占住自己那条连接时，新请求（含 Dashboard）另开新连接，互不阻塞。
const POOL_MAX_IDLE_PER_HOST: usize = 65535;

/// 构建一个带连接超时的 version app 客户端。
///
/// 连接超时设置在 connector 上,保证 TCP/TLS 建立阶段即可快速失败。
/// 响应头等待不在此处设置,因为它需要按请求施加 deadline 且不能影响 body 流。
///
/// 关键决策：用 HTTP/1.1 连接池，不用 h2。
/// hyper-util 的 pool 对 HTTP/2 是 shared 单连接（同一 host 只开一条 h2 连接多路复用），
/// 慢请求占住单连接的 stream 分配权时，新请求会在 checkout 阶段串行排队，
/// 抬 max_concurrent_streams 解决不了单连接串行化。HTTP/1.1 每请求一条独立连接，
/// 慢请求只阻塞自己那条连接，从根上避免 h2c 被慢请求占满。
fn build_version_client(connect_timeout: Duration) -> VersionClient {
    let mut connector = HttpConnector::new();
    // 建立连接阶段挂起会拖垮 gateway,这里给出明确上限。
    connector.set_connect_timeout(connect_timeout_option(connect_timeout));
    Client::builder(TokioExecutor::new())
        .pool_max_idle_per_host(POOL_MAX_IDLE_PER_HOST)
        .build(connector)
}

/// 把 `Duration` 转成 connector 期望的 `Option<Duration>`。
///
/// `Duration::ZERO` 视为禁用连接超时,保持与显式禁用语义一致。
fn connect_timeout_option(timeout: Duration) -> Option<Duration> {
    if timeout.is_zero() {
        None
    } else {
        Some(timeout)
    }
}

/// 切流前独立执行 version app 健康检查。
pub(crate) struct HealthChecker {
    client: VersionClient,
    timeout: Duration,
}

impl HealthChecker {
    /// 创建使用独立连接池的健康检查器。
    pub(crate) fn new(connect_timeout: Duration, timeout: Duration) -> Self {
        Self {
            client: build_version_client(connect_timeout),
            timeout,
        }
    }

    /// 请求健康地址并要求返回 2xx。
    pub(crate) async fn check(&self, health: &str) -> HyperResult<()> {
        let request = http::Request::builder()
            .method(http::Method::GET)
            .uri(health)
            .body(Body::empty())
            .map_err(|error| HyperError::new(format!("build health request failed: {error}")))?;
        let response = tokio::time::timeout(self.timeout, self.client.request(request))
            .await
            .map_err(|_| {
                HyperError::new(format!("health check timed out after {:?}", self.timeout))
            })?
            .map_err(|error| HyperError::new(format!("health check request failed: {error}")))?;
        if !response.status().is_success() {
            return Err(HyperError::new(format!(
                "health check returned {}",
                response.status()
            )));
        }
        Ok(())
    }
}
