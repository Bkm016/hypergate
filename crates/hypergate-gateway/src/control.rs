//! Gateway 生命周期控制入口。
//!
//! [`GatewayControl`] 是 CLI 与 HTTP API 共享的唯一生命周期控制入口。
//! 它持有现有 `manager` / `gateway` / `versions` 的 [`Arc`] 引用,
//! 并用内部 [`Mutex`] 统一维护回滚历史和串行化控制动作,避免并发竞态。
//!
//! 控制动作包括: `switch` / `drain` / `stop` / `rollback` / `reload` / `snapshot`。
//! 其中 `switch` / `drain` / `stop` / `rollback` 接受 [`Option<u64>`] 期望修订号,
//! CLI 传入 `None`,HTTP API 传入具体修订号实现乐观并发控制;
//! 当传入的期望修订号与当前配置修订号不匹配时返回错误。
//!
//! 本模块不实现健康检查,业务语义与原 lifecycle 指令保持一致。

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use hypergate_config::{ConfigManager, RuntimeConfig};
use hypergate_core::{HyperError, HyperResult, VersionId};

use crate::http::Gateway;
use crate::runtime::VersionRegistry;

/// Gateway 生命周期控制入口。
pub(crate) struct GatewayControl {
    /// 配置管理器。
    manager: Arc<ConfigManager<RuntimeConfig>>,
    /// HTTP 网关核心。
    gateway: Arc<Gateway>,
    /// 版本运行态注册表。
    versions: Arc<VersionRegistry>,
    /// 串行化控制动作并维护回滚历史。
    state: Mutex<ControlState>,
    /// 控制器创建时间,用于计算 uptime。
    started_at: Instant,
}

/// 控制器内部可变状态。
struct ControlState {
    /// active 版本回滚历史。
    history: Vec<VersionId>,
}

/// 控制动作失败类型,区分运行态冲突和内部故障。
pub(crate) enum ControlError {
    /// 当前状态拒绝操作,客户端应刷新状态后重新决策。
    Conflict(String),
    /// 内部组件执行失败。
    Internal(HyperError),
}

impl fmt::Display for ControlError {
    /// 渲染 CLI 和服务端日志使用的错误文本。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Internal(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl From<ControlError> for HyperError {
    /// 将结构化控制错误转换成 CLI 使用的通用错误。
    fn from(error: ControlError) -> Self {
        Self::new(error.to_string())
    }
}

/// 控制动作返回类型。
pub(crate) type ControlResult<T> = Result<T, ControlError>;

impl GatewayControl {
    /// 创建生命周期控制入口。
    pub(crate) fn new(
        manager: Arc<ConfigManager<RuntimeConfig>>,
        gateway: Arc<Gateway>,
        versions: Arc<VersionRegistry>,
    ) -> Self {
        Self {
            manager,
            gateway,
            versions,
            state: Mutex::new(ControlState {
                history: Vec::new(),
            }),
            started_at: Instant::now(),
        }
    }

    /// 校验期望修订号是否与当前配置修订号一致。
    /// `None` 表示不校验(CLI 路径),`Some(value)` 表示要求当前修订号必须等于该值。
    fn check_expected(&self, expected: Option<u64>) -> ControlResult<()> {
        match expected {
            None => Ok(()),
            Some(expected_revision) => {
                let current = self.manager.revision().value;
                if current == expected_revision {
                    Ok(())
                } else {
                    Err(ControlError::Conflict(format!(
                        "revision mismatch: expected {expected_revision}, current {current}"
                    )))
                }
            }
        }
    }

    /// 切换 active version,新请求立即进入目标版本。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 切换成功后返回形如 `active=<version>` 的摘要文本。
    pub(crate) fn switch(
        &self,
        next_version: VersionId,
        expected: Option<u64>,
    ) -> ControlResult<ControlOutcome> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(expected)?;
        let summary = self.switch_active(&mut state, next_version, true)?;
        Ok(self.outcome(&state, summary))
    }

    /// 让非 active 版本停止接收新请求(进入 draining)。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 成功后返回形如 `draining=<version>` 的摘要文本。
    pub(crate) fn drain(
        &self,
        version: VersionId,
        expected: Option<u64>,
    ) -> ControlResult<ControlOutcome> {
        let state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(expected)?;
        let config = self.manager.snapshot();
        if config.active_version == version {
            return Err(ControlError::Conflict(
                "active version cannot be drained directly".to_owned(),
            ));
        }
        let runtime = self
            .versions
            .get(&version)
            .map_err(ControlError::Internal)?
            .ok_or_else(|| ControlError::Conflict("version not found".to_owned()))?;
        runtime.drain();
        Ok(self.outcome(&state, format!("draining={}", version.value)))
    }

    /// 在连接清空后停止非 active 版本。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 成功后返回形如 `stopped=<version>` 的摘要文本。
    pub(crate) fn stop(
        &self,
        version: VersionId,
        expected: Option<u64>,
    ) -> ControlResult<ControlOutcome> {
        let state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(expected)?;
        let config = self.manager.snapshot();
        if config.active_version == version {
            return Err(ControlError::Conflict(
                "active version cannot be stopped".to_owned(),
            ));
        }
        let runtime = self
            .versions
            .get(&version)
            .map_err(ControlError::Internal)?
            .ok_or_else(|| ControlError::Conflict("version not found".to_owned()))?;
        runtime
            .stop_idle()
            .map_err(|error| ControlError::Conflict(error.to_string()))?;
        Ok(self.outcome(&state, format!("stopped={}", version.value)))
    }

    /// 切回上一个 active version。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 成功后返回形如 `active=<version>` 的摘要文本。
    pub(crate) fn rollback(&self, expected: Option<u64>) -> ControlResult<ControlOutcome> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(expected)?;
        let previous = state
            .history
            .last()
            .cloned()
            .ok_or_else(|| ControlError::Conflict("rollback history is empty".to_owned()))?;
        let summary = self.switch_active(&mut state, previous, false)?;
        state.history.pop();
        Ok(self.outcome(&state, summary))
    }

    /// 重新加载配置快照并同步 gateway 热路径状态。
    ///
    /// 成功后返回 `reload=ok`。
    pub(crate) fn reload(&self) -> ControlResult<ControlOutcome> {
        let state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        let old = self.manager.snapshot();
        let mut next = old.as_ref().clone();
        next.revision = next.revision.next();
        let prepared = self
            .gateway
            .prepare_active(&next)
            .map_err(ControlError::Internal)?;
        self.manager.apply(next).map_err(ControlError::Internal)?;
        self.gateway.swap_active(prepared);
        Ok(self.outcome(&state, "reload=ok".to_owned()))
    }

    /// 执行 active version 切换并按需记录回滚历史。
    fn switch_active(
        &self,
        state: &mut ControlState,
        next_version: VersionId,
        push_history: bool,
    ) -> ControlResult<String> {
        let old = self.manager.snapshot();
        if !old.versions.contains_key(&next_version) {
            return Err(ControlError::Conflict(
                "version is not configured".to_owned(),
            ));
        }
        if old.active_version == next_version {
            return Ok(format!("active={}", old.active_version.value));
        }
        let mut next = old.as_ref().clone();
        next.revision = next.revision.next();
        next.active_version = next_version.clone();
        let prepared = self
            .gateway
            .prepare_active(&next)
            .map_err(ControlError::Internal)?;
        let old_runtime = self
            .versions
            .get(&old.active_version)
            .map_err(ControlError::Internal)?
            .ok_or_else(|| {
                ControlError::Internal(HyperError::new("active version is not registered"))
            })?;
        let next_runtime = prepared.runtime.clone();
        self.manager
            .validator
            .validate(&next)
            .map_err(ControlError::Internal)?;
        next_runtime.activate();
        // 新版本激活后原子替换热路径,旧版本此时仍可承接已读取旧快照的请求。
        let previous_target = self.gateway.swap_active(prepared);
        let applied = match self.manager.apply(next) {
            Ok(applied) => applied,
            Err(error) => {
                self.gateway.swap_active(previous_target);
                next_runtime.drain();
                return Err(ControlError::Internal(error));
            }
        };
        old_runtime.drain();
        if push_history {
            state.history.push(old.active_version.clone());
        }
        Ok(format!("active={}", applied.active_version.value))
    }

    /// 生成可 serde 序列化的状态快照 DTO。
    ///
    /// 快照字段使用 camelCase,便于后续 HTTP API 直接序列化为 JSON。
    pub(crate) fn snapshot(&self) -> HyperResult<GatewaySnapshot> {
        let state = self
            .state
            .lock()
            .map_err(|_| HyperError::new("control lock poisoned"))?;
        self.snapshot_unlocked(&state)
    }

    /// 在已持有控制锁时生成动作结果。
    fn outcome(&self, state: &ControlState, summary: String) -> ControlOutcome {
        ControlOutcome {
            summary,
            snapshot: self.snapshot_unlocked(state),
        }
    }

    /// 在已持有控制锁时读取一致状态快照。
    fn snapshot_unlocked(&self, state: &ControlState) -> HyperResult<GatewaySnapshot> {
        let config = self.manager.snapshot();
        let snapshots = self.versions.snapshots()?;
        let mut versions: Vec<_> = snapshots
            .into_iter()
            .map(|snapshot| {
                let version = config
                    .versions
                    .get(&VersionId::new(snapshot.id.value.clone()));
                VersionSnapshotDto {
                    id: snapshot.id.value.to_string(),
                    state: snapshot.state.as_str().to_owned(),
                    endpoint: version.map(|version| version.endpoint.clone()),
                    health: version.and_then(|version| version.health.clone()),
                    active_requests: snapshot.active_requests,
                    active_streams: snapshot.active_streams,
                    total_requests: snapshot.total_requests,
                    drain_elapsed_seconds: snapshot.drain_elapsed_secs,
                }
            })
            .collect();
        versions.sort_by(|a, b| a.id.cmp(&b.id));
        let rollback_available = !state.history.is_empty();
        Ok(GatewaySnapshot {
            active_version: config.active_version.value.to_string(),
            revision: config.revision.value,
            rollback_available,
            uptime_seconds: self.started_at.elapsed().as_secs(),
            versions,
        })
    }
}

/// 控制动作结果,包含 CLI 摘要和同一临界区内生成的状态快照。
pub(crate) struct ControlOutcome {
    /// CLI 使用的简短动作摘要。
    pub(crate) summary: String,
    /// HTTP API 返回的动作完成后状态,读取失败不改变动作执行结果。
    pub(crate) snapshot: HyperResult<GatewaySnapshot>,
}

/// Gateway 状态快照 DTO,可序列化为 JSON。
///
/// 所有字段使用 camelCase,供后续 HTTP API 直接序列化。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GatewaySnapshot {
    /// 当前 active 版本标识。
    active_version: String,
    /// 当前配置修订号。
    revision: u64,
    /// 是否存在可回滚的历史版本。
    rollback_available: bool,
    /// 控制器启动至今的秒数。
    uptime_seconds: u64,
    /// 全部版本运行态快照。
    versions: Vec<VersionSnapshotDto>,
}

/// 单个版本运行态快照 DTO。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionSnapshotDto {
    /// 版本标识。
    id: String,
    /// 当前版本状态文本。
    state: String,
    /// Version app 入口地址,运行态未关联配置时为 `None`。
    endpoint: Option<String>,
    /// 健康检查地址,未配置时为 `None`。
    health: Option<String>,
    /// 活跃请求数量。
    active_requests: u64,
    /// 活跃流式连接数量。
    active_streams: u64,
    /// 累计成功创建的请求租约总数。
    total_requests: u64,
    /// draining 已持续秒数,未进入 draining 时为 null。
    drain_elapsed_seconds: Option<u64>,
}
