//! 响应流包装器。用于把版本租约绑定到响应流生命周期。

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::runtime::VersionLease;
use bytes::Bytes;
use futures_util::Stream;

/// 绑定版本租约的 version app 响应流。
pub(crate) struct LeaseStream<S> {
    /// Version app 响应流。
    pub(crate) inner: Pin<Box<S>>,
    /// 请求持有的版本租约。
    pub(crate) lease: Option<VersionLease>,
}

impl<S> Stream for LeaseStream<S>
where
    S: Stream<Item = Result<Bytes, axum::Error>>,
{
    type Item = Result<Bytes, Infallible>;

    /// 转发响应 chunk,并在流结束或出错时释放版本租约。
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(Ok(chunk))),
            Poll::Ready(Some(Err(_))) => {
                self.lease.take();
                Poll::Ready(None)
            }
            Poll::Ready(None) => {
                self.lease.take();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
