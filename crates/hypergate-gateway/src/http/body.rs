//! 请求体生命周期策略。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

/// 反代请求体策略。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProxyBodyPolicy {
    /// 请求体最大字节数。默认值用于阻断异常大请求长时间占用内存和带宽。
    pub(crate) max_request_body_bytes: usize,
}

impl ProxyBodyPolicy {
    /// 默认请求体上限: 64 MiB。
    pub(crate) const DEFAULT_MAX_REQUEST_BODY_BYTES: usize = 64 * 1024 * 1024;
}

impl Default for ProxyBodyPolicy {
    /// 使用框架默认请求体上限。
    fn default() -> Self {
        Self {
            max_request_body_bytes: Self::DEFAULT_MAX_REQUEST_BODY_BYTES,
        }
    }
}

/// 带总量限制的请求体流。
pub(crate) struct LimitedBodyStream<S> {
    /// 下游请求体数据流。
    pub(crate) inner: Pin<Box<S>>,
    /// 请求体策略。
    pub(crate) policy: ProxyBodyPolicy,
    /// 已转发字节数。
    pub(crate) forwarded: usize,
}

impl<S, E> Stream for LimitedBodyStream<S>
where
    S: Stream<Item = Result<Bytes, E>>,
    E: Error + Send + Sync + 'static,
{
    type Item = Result<Bytes, Box<dyn Error + Send + Sync>>;

    /// 转发一个 body chunk,并在累计字节数超过策略上限时中断流。
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => match self.forwarded.checked_add(chunk.len()) {
                Some(total) if total <= self.policy.max_request_body_bytes => {
                    self.forwarded = total;
                    Poll::Ready(Some(Ok(chunk)))
                }
                _ => Poll::Ready(Some(Err(Box::new(RequestBodyLimitExceeded {
                    limit: self.policy.max_request_body_bytes,
                })))),
            },
            Poll::Ready(Some(Err(error))) => Poll::Ready(Some(Err(Box::new(error)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// 请求体超过策略上限时返回给 hyper body stream 的错误。
#[derive(Debug)]
struct RequestBodyLimitExceeded {
    /// 请求体上限。
    limit: usize,
}

impl Display for RequestBodyLimitExceeded {
    /// 输出请求体超限错误。
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "request body exceeds limit: {} bytes", self.limit)
    }
}

impl Error for RequestBodyLimitExceeded {}
