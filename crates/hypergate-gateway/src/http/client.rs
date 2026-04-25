//! Version app HTTP 客户端池。

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
    /// 构建 version app 客户端池。
    pub(crate) fn new() -> Self {
        // 转发热路径使用 hyper client,避免 reqwest 的通用客户端抽象额外开销。
        let unary = Client::builder(TokioExecutor::new()).build_http();
        let streaming = Client::builder(TokioExecutor::new()).build_http();
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
