//! 单版本运行态。

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use hypergate_core::{HyperError, HyperResult, RequestKind, VersionId, VersionState};

use super::VersionLease;

/// 单个版本运行态。
pub(crate) struct VersionRuntime {
    /// 版本标识。
    pub(crate) id: VersionId,
    /// 当前版本状态。
    pub(crate) state: AtomicU8,
    /// 活跃请求数量。
    pub(crate) active_requests: AtomicU64,
    /// 活跃流式连接数量。
    pub(crate) active_streams: AtomicU64,
    /// 进入 draining 的时间。
    pub(crate) drain_started_at: RwLock<Option<Instant>>,
}

impl VersionRuntime {
    /// 创建版本运行态。
    pub(crate) fn new(id: VersionId) -> Self {
        Self {
            id,
            state: AtomicU8::new(encode_state(VersionState::Stopped)),
            active_requests: AtomicU64::new(0),
            active_streams: AtomicU64::new(0),
            drain_started_at: RwLock::new(None),
        }
    }

    /// 判断版本是否可以承接新请求。
    pub(crate) fn accepts_new_requests(&self) -> bool {
        let state = decode_state(self.state.load(Ordering::Acquire));
        state.accepts_new_requests()
    }

    /// 为当前版本创建请求租约。
    pub(crate) fn lease(self: &Arc<Self>, kind: RequestKind) -> HyperResult<VersionLease> {
        if !self.accepts_new_requests() {
            return Err(HyperError::new("version does not accept new requests"));
        }
        // 租约创建成功后立刻计数,后续由 Drop 释放,覆盖短请求和流式连接。
        self.active_requests.fetch_add(1, Ordering::Relaxed);
        if matches!(kind, RequestKind::Stream) {
            self.active_streams.fetch_add(1, Ordering::Relaxed);
        }
        Ok(VersionLease {
            version: self.clone(),
            kind,
        })
    }

    /// 标记版本为 active。
    pub(crate) fn activate(&self) -> HyperResult<()> {
        self.state
            .store(encode_state(VersionState::Active), Ordering::Release);

        let mut drain_started_at = self
            .drain_started_at
            .write()
            .map_err(|_| HyperError::new("version drain lock poisoned"))?;
        *drain_started_at = None;
        Ok(())
    }

    /// 标记版本进入 draining。已有长连接继续持有引用直到结束。
    pub(crate) fn drain(&self) -> HyperResult<()> {
        self.state
            .store(encode_state(VersionState::Draining), Ordering::Release);

        let mut drain_started_at = self
            .drain_started_at
            .write()
            .map_err(|_| HyperError::new("version drain lock poisoned"))?;
        *drain_started_at = Some(Instant::now());
        Ok(())
    }

    /// 无活跃连接时停止版本。
    pub(crate) fn stop_idle(&self) -> HyperResult<()> {
        if !self.is_idle() {
            return Err(HyperError::new("version still has active connections"));
        }
        self.state
            .store(encode_state(VersionState::Stopped), Ordering::Release);

        let mut drain_started_at = self
            .drain_started_at
            .write()
            .map_err(|_| HyperError::new("version drain lock poisoned"))?;
        *drain_started_at = None;
        Ok(())
    }

    /// 活跃连接是否已归零。
    pub(crate) fn is_idle(&self) -> bool {
        self.active_requests.load(Ordering::Relaxed) == 0
            && self.active_streams.load(Ordering::Relaxed) == 0
    }

    /// 读取版本状态快照。
    pub(crate) fn snapshot(&self) -> HyperResult<VersionSnapshot> {
        let state = decode_state(self.state.load(Ordering::Acquire));
        let drain_elapsed_secs = self
            .drain_started_at
            .read()
            .map_err(|_| HyperError::new("version drain lock poisoned"))?
            .map(|started_at| started_at.elapsed().as_secs());
        Ok(VersionSnapshot {
            id: self.id.clone(),
            state,
            active_requests: self.active_requests.load(Ordering::Relaxed),
            active_streams: self.active_streams.load(Ordering::Relaxed),
            drain_elapsed_secs,
        })
    }
}

/// 将状态编码成原子整数,避免请求热路径持有锁。
fn encode_state(state: VersionState) -> u8 {
    match state {
        VersionState::Starting => 0,
        VersionState::Warming => 1,
        VersionState::Active => 2,
        VersionState::Draining => 3,
        VersionState::Stopped => 4,
        VersionState::Failed => 5,
    }
}

/// 将原子状态值还原成领域状态,未知值按 stopped 兜底。
fn decode_state(state: u8) -> VersionState {
    match state {
        0 => VersionState::Starting,
        1 => VersionState::Warming,
        2 => VersionState::Active,
        3 => VersionState::Draining,
        5 => VersionState::Failed,
        _ => VersionState::Stopped,
    }
}

/// 版本状态快照。
#[derive(Debug, Clone)]
pub(crate) struct VersionSnapshot {
    /// 版本标识。
    pub(crate) id: VersionId,
    /// 当前版本状态。
    pub(crate) state: VersionState,
    /// 活跃请求数量。
    pub(crate) active_requests: u64,
    /// 活跃流式连接数量。
    pub(crate) active_streams: u64,
    /// draining 已持续秒数。
    pub(crate) drain_elapsed_secs: Option<u64>,
}
