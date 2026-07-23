//! Version app HTTP 客户端池。

use std::time::Duration;

use axum::body::Body;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use hypergate_core::RequestKind;

/// Version app HTTP 客户端。
pub(crate) type VersionClient = Client<HttpConnector, Body>;

/// 分离普通请求和流式请求的 version app 客户端池。
#[derive(Clone)]
pub(crate) struct VersionClients {
    /// 普通短请求客户端。允许保留空闲连接以提高吞吐。
    pub(crate) unary: VersionClient,
    /// 流式请求客户端。与短请求连接池隔离,避免长连接占用短请求池。
    pub(crate) streaming: VersionClient,
}

impl VersionClients {
    /// 构建 version app 客户端池。`connect_timeout` 仅约束与 version app 的
    /// 连接建立阶段;响应头等待由调用侧按请求施加,避免影响 body 流。
    pub(crate) fn new(connect_timeout: Duration) -> Self {
        // 转发热路径使用 hyper client,避免 reqwest 的通用客户端抽象额外开销。
        let unary = build_client(connect_timeout);
        let streaming = build_client(connect_timeout);
        Self { unary, streaming }
    }

    /// 根据请求类型选择 version app 客户端。
    pub(crate) fn select(&self, kind: RequestKind) -> &VersionClient {
        match kind {
            RequestKind::Unary => &self.unary,
            RequestKind::Stream => &self.streaming,
        }
    }
}

/// 构建一个带连接超时的 version app 客户端。
///
/// 连接超时设置在 connector 上,保证 TCP/TLS 建立阶段即可快速失败。
/// 响应头等待不在此处设置,因为它需要按请求施加 deadline 且不能影响 body 流。
fn build_client(connect_timeout: Duration) -> VersionClient {
    let mut connector = HttpConnector::new();
    // 建立连接阶段挂起会拖垮 gateway,这里给出明确上限。
    connector.set_connect_timeout(connect_timeout_option(connect_timeout));
    Client::builder(TokioExecutor::new()).build(connector)
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
