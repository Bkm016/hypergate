//! 请求租约。租约生命周期直接决定版本活跃连接计数。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use hypergate_core::RequestKind;

use super::VersionRuntime;

/// 请求持有的版本引用。丢弃时自动释放计数。
pub(crate) struct VersionLease {
    /// 被请求绑定的版本。
    pub(crate) version: Arc<VersionRuntime>,
    /// 被请求绑定的连接类型。
    pub(crate) kind: RequestKind,
}

impl Drop for VersionLease {
    /// 释放请求租约时同步扣减版本活跃计数。
    fn drop(&mut self) {
        self.version.active_requests.fetch_sub(1, Ordering::Relaxed);
        if matches!(self.kind, RequestKind::Stream) {
            self.version.active_streams.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
