//! 请求租约。租约生命周期直接决定版本活跃连接计数。

use hypergate_core::RequestKind;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::VersionRuntime;

/// 请求持有的版本引用。丢弃时自动释放计数。
pub(crate) struct VersionLease {
    /// 被请求绑定的版本。
    pub(crate) version: Arc<VersionRuntime>,
    /// 被请求绑定的连接类型。
    pub(crate) kind: RequestKind,
    /// 排水超时后终止该版本残留请求的令牌快照。
    pub(crate) cancel: CancellationToken,
}

impl Drop for VersionLease {
    /// 释放请求租约时同步扣减版本活跃计数。
    fn drop(&mut self) {
        self.version.release(self.kind);
    }
}

impl VersionLease {
    /// 在收到 SSE 或 WebSocket 响应后把普通租约提升为流式租约。
    pub(crate) fn promote_stream(&mut self) {
        if matches!(self.kind, RequestKind::Stream) {
            return;
        }
        self.version.promote_stream();
        self.kind = RequestKind::Stream;
    }

    /// 返回当前租约对应的排水取消令牌。
    pub(crate) fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}
