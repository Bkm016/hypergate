use std::sync::Arc;

/// 运行版本标识。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionId {
    /// 版本名称,例如 v1、v2、stable、canary。
    pub value: Arc<str>,
}

impl VersionId {
    /// 创建版本标识。
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self {
            value: value.into(),
        }
    }
}

/// 运行版本状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionState {
    /// 版本正在启动。
    Starting,
    /// 版本已启动,正在等待健康检查和预热。
    Warming,
    /// 版本可接收新请求。
    Active,
    /// 版本不再接收新请求,只等待既有长连接结束。
    Draining,
    /// 版本已停止。
    Stopped,
    /// 版本启动或运行失败。
    Failed,
}

impl VersionState {
    /// 判断该状态是否允许接收新请求。
    pub fn accepts_new_requests(self) -> bool {
        matches!(self, Self::Active)
    }

    /// 返回稳定的状态文本。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Warming => "warming",
            Self::Active => "active",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}
