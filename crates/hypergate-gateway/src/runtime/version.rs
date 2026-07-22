//! 单版本运行态。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use hypergate_core::{HyperError, HyperResult, RequestKind, VersionId, VersionState};

use super::VersionLease;

/// 打包状态占用的低位数。
const STATE_BITS: u32 = 8;
/// 打包状态的低位掩码。
const STATE_MASK: u64 = (1 << STATE_BITS) - 1;
/// 单个请求租约对应的打包计数增量。
const REQUEST_INCREMENT: u64 = 1 << STATE_BITS;

/// 单个版本运行态。
pub(crate) struct VersionRuntime {
    /// 版本标识。
    pub(crate) id: VersionId,
    /// 当前版本状态和活跃请求数,通过单个 CAS 保证准入与排水原子化。
    state_and_requests: AtomicU64,
    /// 活跃流式连接数量。
    pub(crate) active_streams: AtomicU64,
    /// 累计成功创建的请求租约总数。
    pub(crate) total_requests: AtomicU64,
    /// 进入 draining 的时间。
    pub(crate) drain_started_at: RwLock<Option<Instant>>,
}

impl VersionRuntime {
    /// 创建版本运行态。
    pub(crate) fn new(id: VersionId) -> Self {
        Self {
            id,
            state_and_requests: AtomicU64::new(encode_state(VersionState::Stopped) as u64),
            active_streams: AtomicU64::new(0),
            total_requests: AtomicU64::new(0),
            drain_started_at: RwLock::new(None),
        }
    }

    /// 为当前版本创建请求租约。
    pub(crate) fn lease(self: &Arc<Self>, kind: RequestKind) -> HyperResult<VersionLease> {
        let mut current = self.state_and_requests.load(Ordering::Acquire);
        loop {
            if !unpack_state(current).accepts_new_requests() {
                return Err(HyperError::new("version does not accept new requests"));
            }
            let Some(next) = current.checked_add(REQUEST_INCREMENT) else {
                return Err(HyperError::new("version request counter overflow"));
            };
            match self.state_and_requests.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
        if matches!(kind, RequestKind::Stream) {
            self.active_streams.fetch_add(1, Ordering::Relaxed);
        }
        // 累计成功创建的请求租约总数,用于状态快照和统计。
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        Ok(VersionLease {
            version: self.clone(),
            kind,
        })
    }

    /// 标记版本为 active。
    pub(crate) fn activate(&self) {
        let mut drain_started_at = match self.drain_started_at.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *drain_started_at = None;
        self.set_state(VersionState::Active);
    }

    /// 标记版本进入 draining。已有长连接继续持有引用直到结束。
    pub(crate) fn drain(&self) {
        let mut drain_started_at = match self.drain_started_at.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *drain_started_at = Some(Instant::now());
        self.set_state(VersionState::Draining);
    }

    /// 无活跃连接时停止版本。
    pub(crate) fn stop_idle(&self) -> HyperResult<()> {
        let mut drain_started_at = match self.drain_started_at.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut current = self.state_and_requests.load(Ordering::Acquire);
        loop {
            if unpack_requests(current) != 0 || self.active_streams.load(Ordering::Relaxed) != 0 {
                return Err(HyperError::new("version still has active connections"));
            }
            let next = (current & !STATE_MASK) | encode_state(VersionState::Stopped) as u64;
            match self.state_and_requests.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    *drain_started_at = None;
                    return Ok(());
                }
                Err(observed) => current = observed,
            }
        }
    }

    /// 释放一个请求租约的活跃计数。
    pub(crate) fn release(&self, kind: RequestKind) {
        if matches!(kind, RequestKind::Stream) {
            self.active_streams.fetch_sub(1, Ordering::Relaxed);
        }
        self.state_and_requests
            .fetch_sub(REQUEST_INCREMENT, Ordering::AcqRel);
    }

    /// 读取版本状态快照。
    pub(crate) fn snapshot(&self) -> VersionSnapshot {
        let drain_started_at = match self.drain_started_at.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // 与状态写路径使用相同的“时间锁 -> 原子状态”顺序,避免拼接两个转换时刻。
        let state_and_requests = self.state_and_requests.load(Ordering::Acquire);
        let state = unpack_state(state_and_requests);
        let drain_elapsed_secs = drain_started_at.map(|started_at| started_at.elapsed().as_secs());
        VersionSnapshot {
            id: self.id.clone(),
            state,
            active_requests: unpack_requests(state_and_requests),
            active_streams: self.active_streams.load(Ordering::Relaxed),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            drain_elapsed_secs,
        }
    }

    /// 原子替换打包状态并保留当前活跃请求计数。
    fn set_state(&self, state: VersionState) {
        let encoded = encode_state(state) as u64;
        let mut current = self.state_and_requests.load(Ordering::Acquire);
        loop {
            let next = (current & !STATE_MASK) | encoded;
            match self.state_and_requests.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }
}

/// 从打包值读取版本状态。
fn unpack_state(value: u64) -> VersionState {
    decode_state((value & STATE_MASK) as u8)
}

/// 从打包值读取活跃请求数。
fn unpack_requests(value: u64) -> u64 {
    value >> STATE_BITS
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
    /// 累计成功创建的请求租约总数。
    pub(crate) total_requests: u64,
    /// draining 已持续秒数。
    pub(crate) drain_elapsed_secs: Option<u64>,
}
