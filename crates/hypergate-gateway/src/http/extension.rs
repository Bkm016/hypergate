//! HTTP 请求连接类型识别。

use axum::extract::Request;
use hypergate_core::RequestKind;

/// 请求连接类型识别器扩展点。
pub(crate) trait RequestKindClassifier: Send + Sync {
    /// 判断请求连接类型。
    fn classify(&self, request: &Request) -> RequestKind;
}

/// 默认请求连接类型识别器。
pub(crate) struct DefaultRequestKindClassifier;

impl RequestKindClassifier for DefaultRequestKindClassifier {
    /// 默认把 SSE 请求识别为长连接,其他请求按短请求处理。
    fn classify(&self, request: &Request) -> RequestKind {
        if request
            .headers()
            .get(http::header::UPGRADE)
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
        {
            return RequestKind::Stream;
        }
        if request
            .headers()
            .get(http::header::ACCEPT)
            .is_some_and(|value| {
                value
                    .as_bytes()
                    .windows(17)
                    .any(|w| w == b"text/event-stream")
            })
        {
            return RequestKind::Stream;
        }
        RequestKind::Unary
    }
}
