//! 请求体生命周期策略。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::Stream;

/// 反代请求体策略。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProxyBodyPolicy {
    /// 请求体最大字节数；`None` 表示 HyperGate 不设上限，交由下游版本服务自行校验。
    pub(crate) max_request_body_bytes: Option<usize>,
}

impl Default for ProxyBodyPolicy {
    /// 默认不对请求体大小设限，避免先于下游业务配置拦截合法大请求。
    fn default() -> Self {
        Self {
            max_request_body_bytes: None,
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
    /// 无 Content-Length 请求体超限时通知代理响应映射为 413。
    pub(crate) exceeded: Arc<AtomicBool>,
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
            Poll::Ready(Some(Ok(chunk))) => match self.policy.max_request_body_bytes {
                Some(limit) => match self.forwarded.checked_add(chunk.len()) {
                    Some(total) if total <= limit => {
                        self.forwarded = total;
                        Poll::Ready(Some(Ok(chunk)))
                    }
                    _ => {
                        self.exceeded.store(true, Ordering::Release);
                        Poll::Ready(Some(Err(Box::new(RequestBodyLimitExceeded {
                            limit,
                        }))))
                    }
                },
                None => {
                    self.forwarded = self.forwarded.saturating_add(chunk.len());
                    Poll::Ready(Some(Ok(chunk)))
                }
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
